//! Generated self-contained canonical-CBOR codec from CSIL specification.
//!
//! CSIL is the CBOR Service Interface Language; this codec owns the payload
//! wire (a CBOR map keyed by the verbatim CSIL field name in canonical RFC
//! 8949 order) so the generated types need no serde derive. One
//! `encode_`/`decode_` pair is emitted per record type.
#![allow(dead_code, clippy::vec_init_then_push)]

use super::types::*;

/// A decode failure: the CBOR was malformed or did not match the expected shape.
#[derive(Debug, Clone, PartialEq)]
pub struct CsilCborError(pub String);

impl std::fmt::Display for CsilCborError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CsilCborError {}

/// A minimal canonical-CBOR value tree: a closed set of variants the generated codec
/// builds and walks. A map is an ordered list of pairs, so the encoder controls the
/// wire order of a record's keys explicitly (laid down in canonical order).
#[derive(Debug, Clone, PartialEq)]
pub enum CsilCborValue {
    Uint(u64),
    Int(i64),
    Bool(bool),
    Float(f64),
    Null,
    Text(String),
    Bytes(Vec<u8>),
    Array(Vec<CsilCborValue>),
    Map(Vec<(CsilCborValue, CsilCborValue)>),
    Tag(u64, Box<CsilCborValue>),
}

fn cbor_int(x: i64) -> CsilCborValue {
    CsilCborValue::Int(x)
}
fn cbor_uint(x: u64) -> CsilCborValue {
    CsilCborValue::Uint(x)
}
fn cbor_float(x: f64) -> CsilCborValue {
    CsilCborValue::Float(x)
}
fn cbor_bool(x: bool) -> CsilCborValue {
    CsilCborValue::Bool(x)
}
fn cbor_text(x: &str) -> CsilCborValue {
    CsilCborValue::Text(x.to_string())
}
fn cbor_bytes(x: &[u8]) -> CsilCborValue {
    CsilCborValue::Bytes(x.to_vec())
}

/// Serialize a value tree to canonical CBOR bytes.
fn cbor_encode(v: &CsilCborValue) -> Vec<u8> {
    let mut out = Vec::new();
    cbor_enc(v, &mut out);
    out
}

fn cbor_head(major: u8, n: u64, out: &mut Vec<u8>) {
    let mt = major << 5;
    if n < 24 {
        out.push(mt | n as u8);
    } else if n < 0x100 {
        out.push(mt | 24);
        out.push(n as u8);
    } else if n < 0x10000 {
        out.push(mt | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n < 0x1_0000_0000 {
        out.push(mt | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(mt | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn cbor_enc(v: &CsilCborValue, out: &mut Vec<u8>) {
    match v {
        CsilCborValue::Uint(x) => cbor_head(0, *x, out),
        // A non-negative `Int` rides major type 0 so it is byte-identical to a `Uint`
        // of the same magnitude; only a genuinely negative value uses major type 1.
        CsilCborValue::Int(x) => {
            if *x >= 0 {
                cbor_head(0, *x as u64, out);
            } else {
                cbor_head(1, (-(*x + 1)) as u64, out);
            }
        }
        CsilCborValue::Bool(x) => out.push(if *x { 0xf5 } else { 0xf4 }),
        CsilCborValue::Null => out.push(0xf6),
        CsilCborValue::Float(x) => {
            out.push(0xfb);
            out.extend_from_slice(&x.to_bits().to_be_bytes());
        }
        CsilCborValue::Text(s) => {
            let bytes = s.as_bytes();
            cbor_head(3, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        CsilCborValue::Bytes(b) => {
            cbor_head(2, b.len() as u64, out);
            out.extend_from_slice(b);
        }
        CsilCborValue::Array(items) => {
            cbor_head(4, items.len() as u64, out);
            for item in items {
                cbor_enc(item, out);
            }
        }
        CsilCborValue::Map(entries) => {
            cbor_head(5, entries.len() as u64, out);
            for (k, val) in entries {
                cbor_enc(k, out);
                cbor_enc(val, out);
            }
        }
        CsilCborValue::Tag(num, inner) => {
            cbor_head(6, *num, out);
            cbor_enc(inner, out);
        }
    }
}

/// Parse a full CBOR item and reject trailing bytes, so a payload that is not
/// exactly one value is an error rather than a silently-truncated read.
fn cbor_decode(b: &[u8]) -> Result<CsilCborValue, CsilCborError> {
    let mut pos = 0usize;
    let v = cbor_dec(b, &mut pos, 0)?;
    if pos != b.len() {
        return Err(CsilCborError(format!(
            "csil cbor: {} trailing bytes",
            b.len() - pos
        )));
    }
    Ok(v)
}

fn cbor_read_arg(b: &[u8], pos: &mut usize, low: u8) -> Result<u64, CsilCborError> {
    if low < 24 {
        *pos += 1;
        return Ok(low as u64);
    }
    let width = match low {
        24 => 1usize,
        25 => 2,
        26 => 4,
        27 => 8,
        _ => {
            return Err(CsilCborError(format!(
                "csil cbor: reserved additional info {low}"
            )))
        }
    };
    if *pos >= b.len() || width > b.len() - *pos - 1 {
        return Err(CsilCborError("csil cbor: truncated argument".to_string()));
    }
    let mut v = 0u64;
    for &byte in &b[*pos + 1..*pos + 1 + width] {
        v = (v << 8) | byte as u64;
    }
    *pos += 1 + width;
    Ok(v)
}

/// The most elements a decoded array or map reserves before it reads them. The
/// declared length is checked against the remaining input, but one input byte can
/// become a much larger value, so reserving the full declared length lets a small
/// frame reserve a large multiple of its size at every nesting level. Past this
/// bound, the vector grows only as elements are actually read.
const CSIL_CBOR_PREALLOC_LIMIT: usize = 1024;

fn cbor_dec(b: &[u8], pos: &mut usize, depth: usize) -> Result<CsilCborValue, CsilCborError> {
    if depth > 64 {
        return Err(CsilCborError(
            "csil cbor: nesting limit exceeded".to_string(),
        ));
    }
    if *pos >= b.len() {
        return Err(CsilCborError(
            "csil cbor: unexpected end of input".to_string(),
        ));
    }
    let ib = b[*pos];
    let major = ib >> 5;
    let low = ib & 0x1f;
    if major == 7 {
        return match low {
            20 => {
                *pos += 1;
                Ok(CsilCborValue::Bool(false))
            }
            21 => {
                *pos += 1;
                Ok(CsilCborValue::Bool(true))
            }
            22 | 23 => {
                *pos += 1;
                Ok(CsilCborValue::Null)
            }
            26 => {
                let bits = cbor_read_arg(b, pos, low)?;
                Ok(CsilCborValue::Float(f32::from_bits(bits as u32) as f64))
            }
            27 => {
                let bits = cbor_read_arg(b, pos, low)?;
                Ok(CsilCborValue::Float(f64::from_bits(bits)))
            }
            _ => Err(CsilCborError(format!(
                "csil cbor: unsupported simple value {low}"
            ))),
        };
    }
    let arg = cbor_read_arg(b, pos, low)?;
    match major {
        0 => Ok(CsilCborValue::Uint(arg)),
        1 => {
            if arg > i64::MAX as u64 {
                return Err(CsilCborError(
                    "csil cbor: negative integer out of range".to_string(),
                ));
            }
            Ok(CsilCborValue::Int(-1 - arg as i64))
        }
        2 => {
            if arg > (b.len() - *pos) as u64 {
                return Err(CsilCborError(
                    "csil cbor: truncated byte string".to_string(),
                ));
            }
            let n = arg as usize;
            let slice = b[*pos..*pos + n].to_vec();
            *pos += n;
            Ok(CsilCborValue::Bytes(slice))
        }
        3 => {
            if arg > (b.len() - *pos) as u64 {
                return Err(CsilCborError(
                    "csil cbor: truncated text string".to_string(),
                ));
            }
            let n = arg as usize;
            let s = std::str::from_utf8(&b[*pos..*pos + n])
                .map_err(|e| CsilCborError(format!("csil cbor: invalid utf-8: {e}")))?
                .to_string();
            *pos += n;
            Ok(CsilCborValue::Text(s))
        }
        4 => {
            if arg > (b.len() - *pos) as u64 {
                return Err(CsilCborError(
                    "csil cbor: array length exceeds remaining input".to_string(),
                ));
            }
            let n = arg as usize;
            let mut items = Vec::with_capacity(n.min(CSIL_CBOR_PREALLOC_LIMIT));
            for _ in 0..n {
                items.push(cbor_dec(b, pos, depth + 1)?);
            }
            Ok(CsilCborValue::Array(items))
        }
        5 => {
            if arg > (b.len() - *pos) as u64 {
                return Err(CsilCborError(
                    "csil cbor: map length exceeds remaining input".to_string(),
                ));
            }
            let n = arg as usize;
            let mut entries = Vec::with_capacity(n.min(CSIL_CBOR_PREALLOC_LIMIT));
            for _ in 0..n {
                let k = cbor_dec(b, pos, depth + 1)?;
                let val = cbor_dec(b, pos, depth + 1)?;
                entries.push((k, val));
            }
            Ok(CsilCborValue::Map(entries))
        }
        6 => {
            let inner = cbor_dec(b, pos, depth + 1)?;
            Ok(CsilCborValue::Tag(arg, Box::new(inner)))
        }
        _ => Err(CsilCborError(format!(
            "csil cbor: unexpected major type {major}"
        ))),
    }
}

/// Map a typed slice to a CBOR array via the per-element encoder.
fn cbor_enc_array<E>(xs: &[E], f: impl Fn(&E) -> CsilCborValue) -> CsilCborValue {
    CsilCborValue::Array(xs.iter().map(f).collect())
}

/// Map a typed map to a CBOR map. Rust `HashMap` iteration is unordered, so the inner
/// map's entry order is not canonicalized; the record's own keys (laid down at
/// generation time) are what the cross-language wire contract pins.
fn cbor_enc_map<K, V>(
    m: &std::collections::HashMap<K, V>,
    kf: impl Fn(&K) -> CsilCborValue,
    vf: impl Fn(&V) -> CsilCborValue,
) -> CsilCborValue {
    CsilCborValue::Map(m.iter().map(|(k, v)| (kf(k), vf(v))).collect())
}

fn cbor_dec_array<E>(
    v: &CsilCborValue,
    f: impl Fn(&CsilCborValue) -> Result<E, CsilCborError>,
) -> Result<Vec<E>, CsilCborError> {
    cbor_as_array(v)?.iter().map(f).collect()
}

fn cbor_dec_map<K: std::cmp::Eq + std::hash::Hash, V>(
    v: &CsilCborValue,
    kf: impl Fn(&CsilCborValue) -> Result<K, CsilCborError>,
    vf: impl Fn(&CsilCborValue) -> Result<V, CsilCborError>,
) -> Result<std::collections::HashMap<K, V>, CsilCborError> {
    let entries = cbor_as_map(v)?;
    let mut out = std::collections::HashMap::with_capacity(entries.len());
    for (k, val) in entries {
        out.insert(kf(k)?, vf(val)?);
    }
    Ok(out)
}

fn cbor_map_get<'a>(v: &'a CsilCborValue, key: &str) -> Option<&'a CsilCborValue> {
    if let CsilCborValue::Map(entries) = v {
        for (k, val) in entries {
            if matches!(k, CsilCborValue::Text(name) if name == key) {
                return Some(val);
            }
        }
    }
    None
}

fn cbor_expect_value(v: &CsilCborValue, expected: &CsilCborValue) -> Result<(), CsilCborError> {
    if v == expected {
        Ok(())
    } else {
        Err(CsilCborError(format!(
            "csil cbor: expected literal {expected:?}, got {v:?}"
        )))
    }
}

fn cbor_require<'a>(v: &'a CsilCborValue, key: &str) -> Result<&'a CsilCborValue, CsilCborError> {
    cbor_map_get(v, key).ok_or_else(|| CsilCborError(format!("csil cbor: missing field {key:?}")))
}

fn cbor_as_i64(v: &CsilCborValue) -> Result<i64, CsilCborError> {
    match v {
        CsilCborValue::Uint(x) => i64::try_from(*x)
            .map_err(|_| CsilCborError("csil cbor: integer overflows i64".to_string())),
        CsilCborValue::Int(x) => Ok(*x),
        _ => Err(CsilCborError("csil cbor: expected integer".to_string())),
    }
}

fn cbor_as_u64(v: &CsilCborValue) -> Result<u64, CsilCborError> {
    match v {
        CsilCborValue::Uint(x) => Ok(*x),
        CsilCborValue::Int(x) if *x >= 0 => Ok(*x as u64),
        CsilCborValue::Int(_) => Err(CsilCborError(
            "csil cbor: negative integer where unsigned expected".to_string(),
        )),
        _ => Err(CsilCborError(
            "csil cbor: expected unsigned integer".to_string(),
        )),
    }
}

fn cbor_as_f64(v: &CsilCborValue) -> Result<f64, CsilCborError> {
    match v {
        CsilCborValue::Float(x) => Ok(*x),
        CsilCborValue::Uint(x) => Ok(*x as f64),
        CsilCborValue::Int(x) => Ok(*x as f64),
        _ => Err(CsilCborError("csil cbor: expected float".to_string())),
    }
}

fn cbor_as_bool(v: &CsilCborValue) -> Result<bool, CsilCborError> {
    match v {
        CsilCborValue::Bool(b) => Ok(*b),
        _ => Err(CsilCborError("csil cbor: expected bool".to_string())),
    }
}

fn cbor_as_text(v: &CsilCborValue) -> Result<String, CsilCborError> {
    match v {
        CsilCborValue::Text(s) => Ok(s.clone()),
        _ => Err(CsilCborError("csil cbor: expected text".to_string())),
    }
}

fn cbor_as_bytes(v: &CsilCborValue) -> Result<Vec<u8>, CsilCborError> {
    match v {
        CsilCborValue::Bytes(b) => Ok(b.clone()),
        _ => Err(CsilCborError("csil cbor: expected byte string".to_string())),
    }
}

fn cbor_as_array(v: &CsilCborValue) -> Result<&[CsilCborValue], CsilCborError> {
    match v {
        CsilCborValue::Array(a) => Ok(a),
        _ => Err(CsilCborError("csil cbor: expected array".to_string())),
    }
}

fn cbor_as_map(v: &CsilCborValue) -> Result<&[(CsilCborValue, CsilCborValue)], CsilCborError> {
    match v {
        CsilCborValue::Map(m) => Ok(m),
        _ => Err(CsilCborError("csil cbor: expected map".to_string())),
    }
}

/// Build the canonical CBOR value tree for a Ctx.
fn csil_enc_ctx(csil_v: &Ctx) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("request_id"), cbor_text(&csil_v.request_id)));
    if let Some(csil_inner) = &csil_v.deadline_ms {
        csil_entries.push((cbor_text("deadline_ms"), cbor_uint(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.contract_digest {
        csil_entries.push((cbor_text("contract_digest"), cbor_bytes(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.contract_version {
        csil_entries.push((cbor_text("contract_version"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Ctx from a decoded CBOR value tree.
fn csil_dec_ctx(csil_root: &CsilCborValue) -> Result<Ctx, CsilCborError> {
    let request_id = {
        let csil_field = cbor_require(csil_root, "request_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let deadline_ms = match cbor_map_get(csil_root, "deadline_ms") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let contract_version = match cbor_map_get(csil_root, "contract_version") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let contract_digest = match cbor_map_get(csil_root, "contract_digest") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bytes;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Ctx {
        request_id,
        deadline_ms,
        contract_version,
        contract_digest,
    })
}

/// Encode a Ctx to canonical CSIL CBOR bytes.
pub fn encode_ctx(csil_v: &Ctx) -> Vec<u8> {
    cbor_encode(&csil_enc_ctx(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Ctx.
pub fn decode_ctx(csil_data: &[u8]) -> Result<Ctx, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_ctx(&csil_root)
}

/// Build the canonical CBOR value tree for a ServiceError.
fn csil_enc_service_error(csil_v: &ServiceError) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("code"), cbor_uint(csil_v.code)));
    csil_entries.push((cbor_text("message"), cbor_text(&csil_v.message)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ServiceError from a decoded CBOR value tree.
fn csil_dec_service_error(csil_root: &CsilCborValue) -> Result<ServiceError, CsilCborError> {
    let code = {
        let csil_field = cbor_require(csil_root, "code")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let message = {
        let csil_field = cbor_require(csil_root, "message")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(ServiceError { code, message })
}

/// Encode a ServiceError to canonical CSIL CBOR bytes.
pub fn encode_service_error(csil_v: &ServiceError) -> Vec<u8> {
    cbor_encode(&csil_enc_service_error(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ServiceError.
pub fn decode_service_error(csil_data: &[u8]) -> Result<ServiceError, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_service_error(&csil_root)
}

/// Build the canonical CBOR value tree for a Ok.
fn csil_enc_ok(csil_v: &Ok) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((cbor_text("ok"), cbor_bool(csil_v.ok)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Ok from a decoded CBOR value tree.
fn csil_dec_ok(csil_root: &CsilCborValue) -> Result<Ok, CsilCborError> {
    let ok = {
        let csil_field = cbor_require(csil_root, "ok")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(Ok { ok })
}

/// Encode a Ok to canonical CSIL CBOR bytes.
pub fn encode_ok(csil_v: &Ok) -> Vec<u8> {
    cbor_encode(&csil_enc_ok(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Ok.
pub fn decode_ok(csil_data: &[u8]) -> Result<Ok, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_ok(&csil_root)
}

/// Build the canonical CBOR value tree for a Empty.
fn csil_enc_empty(csil_v: &Empty) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Empty from a decoded CBOR value tree.
fn csil_dec_empty(csil_root: &CsilCborValue) -> Result<Empty, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    Ok(Empty { ctx })
}

/// Encode a Empty to canonical CSIL CBOR bytes.
pub fn encode_empty(csil_v: &Empty) -> Vec<u8> {
    cbor_encode(&csil_enc_empty(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Empty.
pub fn decode_empty(csil_data: &[u8]) -> Result<Empty, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_empty(&csil_root)
}

/// Build the canonical CBOR value tree for a Identity.
fn csil_enc_identity(csil_v: &Identity) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("domain"), cbor_text(&csil_v.domain)));
    if let Some(csil_inner) = &csil_v.handle {
        csil_entries.push((cbor_text("handle"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("user_id"), cbor_text(&csil_v.user_id)));
    if let Some(csil_inner) = &csil_v.display_name {
        csil_entries.push((cbor_text("display_name"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Identity from a decoded CBOR value tree.
fn csil_dec_identity(csil_root: &CsilCborValue) -> Result<Identity, CsilCborError> {
    let user_id = {
        let csil_field = cbor_require(csil_root, "user_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let domain = {
        let csil_field = cbor_require(csil_root, "domain")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let handle = match cbor_map_get(csil_root, "handle") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let display_name = match cbor_map_get(csil_root, "display_name") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Identity {
        user_id,
        domain,
        handle,
        display_name,
    })
}

/// Encode a Identity to canonical CSIL CBOR bytes.
pub fn encode_identity(csil_v: &Identity) -> Vec<u8> {
    cbor_encode(&csil_enc_identity(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Identity.
pub fn decode_identity(csil_data: &[u8]) -> Result<Identity, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_identity(&csil_root)
}

/// Build the canonical CBOR value tree for a RepoRef.
fn csil_enc_repo_ref(csil_v: &RepoRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.instance {
        csil_entries.push((cbor_text("instance"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RepoRef from a decoded CBOR value tree.
fn csil_dec_repo_ref(csil_root: &CsilCborValue) -> Result<RepoRef, CsilCborError> {
    let instance = match cbor_map_get(csil_root, "instance") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(RepoRef {
        instance,
        org,
        name,
    })
}

/// Encode a RepoRef to canonical CSIL CBOR bytes.
pub fn encode_repo_ref(csil_v: &RepoRef) -> Vec<u8> {
    cbor_encode(&csil_enc_repo_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RepoRef.
pub fn decode_repo_ref(csil_data: &[u8]) -> Result<RepoRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_repo_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a OrgRef.
fn csil_enc_org_ref(csil_v: &OrgRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a OrgRef from a decoded CBOR value tree.
fn csil_dec_org_ref(csil_root: &CsilCborValue) -> Result<OrgRef, CsilCborError> {
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(OrgRef { org })
}

/// Encode a OrgRef to canonical CSIL CBOR bytes.
pub fn encode_org_ref(csil_v: &OrgRef) -> Vec<u8> {
    cbor_encode(&csil_enc_org_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a OrgRef.
pub fn decode_org_ref(csil_data: &[u8]) -> Result<OrgRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_org_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a OrgRepoRef.
fn csil_enc_org_repo_ref(csil_v: &OrgRepoRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.repo {
        csil_entries.push((cbor_text("repo"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a OrgRepoRef from a decoded CBOR value tree.
fn csil_dec_org_repo_ref(csil_root: &CsilCborValue) -> Result<OrgRepoRef, CsilCborError> {
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let repo = match cbor_map_get(csil_root, "repo") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(OrgRepoRef { org, repo })
}

/// Encode a OrgRepoRef to canonical CSIL CBOR bytes.
pub fn encode_org_repo_ref(csil_v: &OrgRepoRef) -> Vec<u8> {
    cbor_encode(&csil_enc_org_repo_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a OrgRepoRef.
pub fn decode_org_repo_ref(csil_data: &[u8]) -> Result<OrgRepoRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_org_repo_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a Page.
fn csil_enc_page(csil_v: &Page) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    if let Some(csil_inner) = &csil_v.limit {
        csil_entries.push((cbor_text("limit"), cbor_uint(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.cursor {
        csil_entries.push((cbor_text("cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Page from a decoded CBOR value tree.
fn csil_dec_page(csil_root: &CsilCborValue) -> Result<Page, CsilCborError> {
    let cursor = match cbor_map_get(csil_root, "cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let limit = match cbor_map_get(csil_root, "limit") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Page { cursor, limit })
}

/// Encode a Page to canonical CSIL CBOR bytes.
pub fn encode_page(csil_v: &Page) -> Vec<u8> {
    cbor_encode(&csil_enc_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Page.
pub fn decode_page(csil_data: &[u8]) -> Result<Page, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_page(&csil_root)
}

/// Build the canonical CBOR value tree for a ServerInfo.
fn csil_enc_server_info(csil_v: &ServerInfo) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("version"), cbor_text(&csil_v.version)));
    csil_entries.push((
        cbor_text("features"),
        cbor_enc_array(&csil_v.features, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("public_url"), cbor_text(&csil_v.public_url)));
    csil_entries.push((
        cbor_text("contract_digest"),
        cbor_bytes(&csil_v.contract_digest),
    ));
    csil_entries.push((
        cbor_text("instance_domain"),
        cbor_text(&csil_v.instance_domain),
    ));
    csil_entries.push((
        cbor_text("contract_version"),
        cbor_text(&csil_v.contract_version),
    ));
    csil_entries.push((
        cbor_text("instance_identity"),
        cbor_text(&csil_v.instance_identity),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ServerInfo from a decoded CBOR value tree.
fn csil_dec_server_info(csil_root: &CsilCborValue) -> Result<ServerInfo, CsilCborError> {
    let version = {
        let csil_field = cbor_require(csil_root, "version")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let contract_version = {
        let csil_field = cbor_require(csil_root, "contract_version")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let contract_digest = {
        let csil_field = cbor_require(csil_root, "contract_digest")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let features = {
        let csil_field = cbor_require(csil_root, "features")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let instance_domain = {
        let csil_field = cbor_require(csil_root, "instance_domain")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let public_url = {
        let csil_field = cbor_require(csil_root, "public_url")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let instance_identity = {
        let csil_field = cbor_require(csil_root, "instance_identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(ServerInfo {
        version,
        contract_version,
        contract_digest,
        features,
        instance_domain,
        public_url,
        instance_identity,
    })
}

/// Encode a ServerInfo to canonical CSIL CBOR bytes.
pub fn encode_server_info(csil_v: &ServerInfo) -> Vec<u8> {
    cbor_encode(&csil_enc_server_info(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ServerInfo.
pub fn decode_server_info(csil_data: &[u8]) -> Result<ServerInfo, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_server_info(&csil_root)
}

/// Build the canonical CBOR value tree for a BeginLoginRequest.
fn csil_enc_begin_login_request(csil_v: &BeginLoginRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.return_to {
        csil_entries.push((cbor_text("return_to"), cbor_text(csil_inner)));
    }
    csil_entries.push((
        cbor_text("identity_input"),
        cbor_text(&csil_v.identity_input),
    ));
    if let Some(csil_inner) = &csil_v.device_user_code {
        csil_entries.push((cbor_text("device_user_code"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BeginLoginRequest from a decoded CBOR value tree.
fn csil_dec_begin_login_request(
    csil_root: &CsilCborValue,
) -> Result<BeginLoginRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let identity_input = {
        let csil_field = cbor_require(csil_root, "identity_input")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let return_to = match cbor_map_get(csil_root, "return_to") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let device_user_code = match cbor_map_get(csil_root, "device_user_code") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(BeginLoginRequest {
        ctx,
        identity_input,
        return_to,
        device_user_code,
    })
}

/// Encode a BeginLoginRequest to canonical CSIL CBOR bytes.
pub fn encode_begin_login_request(csil_v: &BeginLoginRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_begin_login_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BeginLoginRequest.
pub fn decode_begin_login_request(csil_data: &[u8]) -> Result<BeginLoginRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_begin_login_request(&csil_root)
}

/// Build the canonical CBOR value tree for a BeginLoginResponse.
fn csil_enc_begin_login_response(csil_v: &BeginLoginResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("expires_at"), cbor_int(csil_v.expires_at)));
    csil_entries.push((cbor_text("redirect_url"), cbor_text(&csil_v.redirect_url)));
    csil_entries.push((cbor_text("attempt_token"), cbor_text(&csil_v.attempt_token)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BeginLoginResponse from a decoded CBOR value tree.
fn csil_dec_begin_login_response(
    csil_root: &CsilCborValue,
) -> Result<BeginLoginResponse, CsilCborError> {
    let redirect_url = {
        let csil_field = cbor_require(csil_root, "redirect_url")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let attempt_token = {
        let csil_field = cbor_require(csil_root, "attempt_token")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let expires_at = {
        let csil_field = cbor_require(csil_root, "expires_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(BeginLoginResponse {
        redirect_url,
        attempt_token,
        expires_at,
    })
}

/// Encode a BeginLoginResponse to canonical CSIL CBOR bytes.
pub fn encode_begin_login_response(csil_v: &BeginLoginResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_begin_login_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BeginLoginResponse.
pub fn decode_begin_login_response(csil_data: &[u8]) -> Result<BeginLoginResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_begin_login_response(&csil_root)
}

/// Build the canonical CBOR value tree for a CompleteLoginRequest.
fn csil_enc_complete_login_request(csil_v: &CompleteLoginRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("attempt_token"), cbor_text(&csil_v.attempt_token)));
    csil_entries.push((
        cbor_text("encrypted_token"),
        cbor_text(&csil_v.encrypted_token),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CompleteLoginRequest from a decoded CBOR value tree.
fn csil_dec_complete_login_request(
    csil_root: &CsilCborValue,
) -> Result<CompleteLoginRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let attempt_token = {
        let csil_field = cbor_require(csil_root, "attempt_token")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let encrypted_token = {
        let csil_field = cbor_require(csil_root, "encrypted_token")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(CompleteLoginRequest {
        ctx,
        attempt_token,
        encrypted_token,
    })
}

/// Encode a CompleteLoginRequest to canonical CSIL CBOR bytes.
pub fn encode_complete_login_request(csil_v: &CompleteLoginRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_complete_login_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CompleteLoginRequest.
pub fn decode_complete_login_request(
    csil_data: &[u8],
) -> Result<CompleteLoginRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_complete_login_request(&csil_root)
}

/// Build the canonical CBOR value tree for a Session.
fn csil_enc_session(csil_v: &Session) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("identity"), csil_enc_identity(&csil_v.identity)));
    if let Some(csil_inner) = &csil_v.return_to {
        csil_entries.push((cbor_text("return_to"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("expires_at"), cbor_int(csil_v.expires_at)));
    csil_entries.push((cbor_text("session_token"), cbor_text(&csil_v.session_token)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Session from a decoded CBOR value tree.
fn csil_dec_session(csil_root: &CsilCborValue) -> Result<Session, CsilCborError> {
    let session_token = {
        let csil_field = cbor_require(csil_root, "session_token")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let expires_at = {
        let csil_field = cbor_require(csil_root, "expires_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let return_to = match cbor_map_get(csil_root, "return_to") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Session {
        session_token,
        identity,
        expires_at,
        return_to,
    })
}

/// Encode a Session to canonical CSIL CBOR bytes.
pub fn encode_session(csil_v: &Session) -> Vec<u8> {
    cbor_encode(&csil_enc_session(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Session.
pub fn decode_session(csil_data: &[u8]) -> Result<Session, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_session(&csil_root)
}

/// Build the canonical CBOR value tree for a CreateTokenRequest.
fn csil_enc_create_token_request(csil_v: &CreateTokenRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    if let Some(csil_inner) = &csil_v.expires_at {
        csil_entries.push((cbor_text("expires_at"), cbor_int(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.client_label {
        csil_entries.push((cbor_text("client_label"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CreateTokenRequest from a decoded CBOR value tree.
fn csil_dec_create_token_request(
    csil_root: &CsilCborValue,
) -> Result<CreateTokenRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let client_label = match cbor_map_get(csil_root, "client_label") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let expires_at = match cbor_map_get(csil_root, "expires_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CreateTokenRequest {
        ctx,
        name,
        scopes,
        client_label,
        expires_at,
    })
}

/// Encode a CreateTokenRequest to canonical CSIL CBOR bytes.
pub fn encode_create_token_request(csil_v: &CreateTokenRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_create_token_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CreateTokenRequest.
pub fn decode_create_token_request(csil_data: &[u8]) -> Result<CreateTokenRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_create_token_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ApiToken.
fn csil_enc_api_token(csil_v: &ApiToken) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    if let Some(csil_inner) = &csil_v.secret {
        csil_entries.push((cbor_text("secret"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    if let Some(csil_inner) = &csil_v.expires_at {
        csil_entries.push((cbor_text("expires_at"), cbor_int(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.client_label {
        csil_entries.push((cbor_text("client_label"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ApiToken from a decoded CBOR value tree.
fn csil_dec_api_token(csil_root: &CsilCborValue) -> Result<ApiToken, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let client_label = match cbor_map_get(csil_root, "client_label") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let expires_at = match cbor_map_get(csil_root, "expires_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let secret = match cbor_map_get(csil_root, "secret") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ApiToken {
        id,
        name,
        scopes,
        client_label,
        created_at,
        expires_at,
        secret,
    })
}

/// Encode a ApiToken to canonical CSIL CBOR bytes.
pub fn encode_api_token(csil_v: &ApiToken) -> Vec<u8> {
    cbor_encode(&csil_enc_api_token(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ApiToken.
pub fn decode_api_token(csil_data: &[u8]) -> Result<ApiToken, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_api_token(&csil_root)
}

/// Build the canonical CBOR value tree for a RevokeTokenRequest.
fn csil_enc_revoke_token_request(csil_v: &RevokeTokenRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("token_id"), cbor_text(&csil_v.token_id)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RevokeTokenRequest from a decoded CBOR value tree.
fn csil_dec_revoke_token_request(
    csil_root: &CsilCborValue,
) -> Result<RevokeTokenRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let token_id = {
        let csil_field = cbor_require(csil_root, "token_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(RevokeTokenRequest { ctx, token_id })
}

/// Encode a RevokeTokenRequest to canonical CSIL CBOR bytes.
pub fn encode_revoke_token_request(csil_v: &RevokeTokenRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_revoke_token_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RevokeTokenRequest.
pub fn decode_revoke_token_request(csil_data: &[u8]) -> Result<RevokeTokenRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_revoke_token_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListTokensResponse.
fn csil_enc_list_tokens_response(csil_v: &ListTokensResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("tokens"),
        cbor_enc_array(&csil_v.tokens, csil_enc_api_token),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListTokensResponse from a decoded CBOR value tree.
fn csil_dec_list_tokens_response(
    csil_root: &CsilCborValue,
) -> Result<ListTokensResponse, CsilCborError> {
    let tokens = {
        let csil_field = cbor_require(csil_root, "tokens")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_api_token);
        csil_decode(csil_field)?
    };
    Ok(ListTokensResponse { tokens })
}

/// Encode a ListTokensResponse to canonical CSIL CBOR bytes.
pub fn encode_list_tokens_response(csil_v: &ListTokensResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_tokens_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListTokensResponse.
pub fn decode_list_tokens_response(csil_data: &[u8]) -> Result<ListTokensResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_tokens_response(&csil_root)
}

/// Build the canonical CBOR value tree for a WhoAmI.
fn csil_enc_who_am_i(csil_v: &WhoAmI) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("identity"), csil_enc_identity(&csil_v.identity)));
    if let Some(csil_inner) = &csil_v.acting_via {
        csil_entries.push((cbor_text("acting_via"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.expires_at {
        csil_entries.push((cbor_text("expires_at"), cbor_int(*csil_inner)));
    }
    csil_entries.push((
        cbor_text("credential_kind"),
        cbor_text(&csil_v.credential_kind),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a WhoAmI from a decoded CBOR value tree.
fn csil_dec_who_am_i(csil_root: &CsilCborValue) -> Result<WhoAmI, CsilCborError> {
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let credential_kind = {
        let csil_field = cbor_require(csil_root, "credential_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let acting_via = match cbor_map_get(csil_root, "acting_via") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let expires_at = match cbor_map_get(csil_root, "expires_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(WhoAmI {
        identity,
        scopes,
        credential_kind,
        acting_via,
        expires_at,
    })
}

/// Encode a WhoAmI to canonical CSIL CBOR bytes.
pub fn encode_who_am_i(csil_v: &WhoAmI) -> Vec<u8> {
    cbor_encode(&csil_enc_who_am_i(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a WhoAmI.
pub fn decode_who_am_i(csil_data: &[u8]) -> Result<WhoAmI, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_who_am_i(&csil_root)
}

/// Build the canonical CBOR value tree for a BeginDeviceRequest.
fn csil_enc_begin_device_request(csil_v: &BeginDeviceRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("client_label"), cbor_text(&csil_v.client_label)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BeginDeviceRequest from a decoded CBOR value tree.
fn csil_dec_begin_device_request(
    csil_root: &CsilCborValue,
) -> Result<BeginDeviceRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let client_label = {
        let csil_field = cbor_require(csil_root, "client_label")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(BeginDeviceRequest {
        ctx,
        client_label,
        scopes,
    })
}

/// Encode a BeginDeviceRequest to canonical CSIL CBOR bytes.
pub fn encode_begin_device_request(csil_v: &BeginDeviceRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_begin_device_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BeginDeviceRequest.
pub fn decode_begin_device_request(csil_data: &[u8]) -> Result<BeginDeviceRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_begin_device_request(&csil_root)
}

/// Build the canonical CBOR value tree for a DeviceCode.
fn csil_enc_device_code(csil_v: &DeviceCode) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("user_code"), cbor_text(&csil_v.user_code)));
    csil_entries.push((cbor_text("expires_at"), cbor_int(csil_v.expires_at)));
    csil_entries.push((cbor_text("device_code"), cbor_text(&csil_v.device_code)));
    csil_entries.push((
        cbor_text("poll_interval_ms"),
        cbor_uint(csil_v.poll_interval_ms),
    ));
    csil_entries.push((
        cbor_text("verification_url"),
        cbor_text(&csil_v.verification_url),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DeviceCode from a decoded CBOR value tree.
fn csil_dec_device_code(csil_root: &CsilCborValue) -> Result<DeviceCode, CsilCborError> {
    let device_code = {
        let csil_field = cbor_require(csil_root, "device_code")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let user_code = {
        let csil_field = cbor_require(csil_root, "user_code")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let verification_url = {
        let csil_field = cbor_require(csil_root, "verification_url")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let expires_at = {
        let csil_field = cbor_require(csil_root, "expires_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let poll_interval_ms = {
        let csil_field = cbor_require(csil_root, "poll_interval_ms")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    Ok(DeviceCode {
        device_code,
        user_code,
        verification_url,
        expires_at,
        poll_interval_ms,
    })
}

/// Encode a DeviceCode to canonical CSIL CBOR bytes.
pub fn encode_device_code(csil_v: &DeviceCode) -> Vec<u8> {
    cbor_encode(&csil_enc_device_code(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DeviceCode.
pub fn decode_device_code(csil_data: &[u8]) -> Result<DeviceCode, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_device_code(&csil_root)
}

/// Build the canonical CBOR value tree for a DevicePollRequest.
fn csil_enc_device_poll_request(csil_v: &DevicePollRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("device_code"), cbor_text(&csil_v.device_code)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DevicePollRequest from a decoded CBOR value tree.
fn csil_dec_device_poll_request(
    csil_root: &CsilCborValue,
) -> Result<DevicePollRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let device_code = {
        let csil_field = cbor_require(csil_root, "device_code")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(DevicePollRequest { ctx, device_code })
}

/// Encode a DevicePollRequest to canonical CSIL CBOR bytes.
pub fn encode_device_poll_request(csil_v: &DevicePollRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_device_poll_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DevicePollRequest.
pub fn decode_device_poll_request(csil_data: &[u8]) -> Result<DevicePollRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_device_poll_request(&csil_root)
}

/// Build the canonical CBOR value tree for a DevicePollResponse.
fn csil_enc_device_poll_response(csil_v: &DevicePollResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    if let Some(csil_inner) = &csil_v.token {
        csil_entries.push((cbor_text("token"), csil_enc_api_token(csil_inner)));
    }
    csil_entries.push((cbor_text("pending"), cbor_bool(csil_v.pending)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DevicePollResponse from a decoded CBOR value tree.
fn csil_dec_device_poll_response(
    csil_root: &CsilCborValue,
) -> Result<DevicePollResponse, CsilCborError> {
    let pending = {
        let csil_field = cbor_require(csil_root, "pending")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let token = match cbor_map_get(csil_root, "token") {
        Some(csil_field) => {
            let csil_decode = csil_dec_api_token;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(DevicePollResponse { pending, token })
}

/// Encode a DevicePollResponse to canonical CSIL CBOR bytes.
pub fn encode_device_poll_response(csil_v: &DevicePollResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_device_poll_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DevicePollResponse.
pub fn decode_device_poll_response(csil_data: &[u8]) -> Result<DevicePollResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_device_poll_response(&csil_root)
}

/// Build the canonical CBOR value tree for a DeviceApproveRequest.
fn csil_enc_device_approve_request(csil_v: &DeviceApproveRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("approve"), cbor_bool(csil_v.approve)));
    csil_entries.push((cbor_text("user_code"), cbor_text(&csil_v.user_code)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DeviceApproveRequest from a decoded CBOR value tree.
fn csil_dec_device_approve_request(
    csil_root: &CsilCborValue,
) -> Result<DeviceApproveRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let user_code = {
        let csil_field = cbor_require(csil_root, "user_code")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let approve = {
        let csil_field = cbor_require(csil_root, "approve")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(DeviceApproveRequest {
        ctx,
        user_code,
        approve,
    })
}

/// Encode a DeviceApproveRequest to canonical CSIL CBOR bytes.
pub fn encode_device_approve_request(csil_v: &DeviceApproveRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_device_approve_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DeviceApproveRequest.
pub fn decode_device_approve_request(
    csil_data: &[u8],
) -> Result<DeviceApproveRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_device_approve_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ActAsExchangeRequest.
fn csil_enc_act_as_exchange_request(csil_v: &ActAsExchangeRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((
        cbor_text("credential_cbor"),
        cbor_bytes(&csil_v.credential_cbor),
    ));
    csil_entries.push((
        cbor_text("requested_scopes"),
        cbor_enc_array(&csil_v.requested_scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ActAsExchangeRequest from a decoded CBOR value tree.
fn csil_dec_act_as_exchange_request(
    csil_root: &CsilCborValue,
) -> Result<ActAsExchangeRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let credential_cbor = {
        let csil_field = cbor_require(csil_root, "credential_cbor")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let requested_scopes = {
        let csil_field = cbor_require(csil_root, "requested_scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(ActAsExchangeRequest {
        ctx,
        credential_cbor,
        requested_scopes,
    })
}

/// Encode a ActAsExchangeRequest to canonical CSIL CBOR bytes.
pub fn encode_act_as_exchange_request(csil_v: &ActAsExchangeRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_act_as_exchange_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ActAsExchangeRequest.
pub fn decode_act_as_exchange_request(
    csil_data: &[u8],
) -> Result<ActAsExchangeRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_act_as_exchange_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ScopedBearer.
fn csil_enc_scoped_bearer(csil_v: &ScopedBearer) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("bearer"), cbor_text(&csil_v.bearer)));
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("subject"), csil_enc_identity(&csil_v.subject)));
    csil_entries.push((cbor_text("acting_via"), cbor_text(&csil_v.acting_via)));
    csil_entries.push((cbor_text("expires_at"), cbor_int(csil_v.expires_at)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ScopedBearer from a decoded CBOR value tree.
fn csil_dec_scoped_bearer(csil_root: &CsilCborValue) -> Result<ScopedBearer, CsilCborError> {
    let bearer = {
        let csil_field = cbor_require(csil_root, "bearer")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let expires_at = {
        let csil_field = cbor_require(csil_root, "expires_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let subject = {
        let csil_field = cbor_require(csil_root, "subject")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let acting_via = {
        let csil_field = cbor_require(csil_root, "acting_via")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(ScopedBearer {
        bearer,
        scopes,
        expires_at,
        subject,
        acting_via,
    })
}

/// Encode a ScopedBearer to canonical CSIL CBOR bytes.
pub fn encode_scoped_bearer(csil_v: &ScopedBearer) -> Vec<u8> {
    cbor_encode(&csil_enc_scoped_bearer(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ScopedBearer.
pub fn decode_scoped_bearer(csil_data: &[u8]) -> Result<ScopedBearer, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_scoped_bearer(&csil_root)
}

/// Build the canonical CBOR value tree for a ScopeSetRequest.
fn csil_enc_scope_set_request(csil_v: &ScopeSetRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("grantee_cbor"), cbor_bytes(&csil_v.grantee_cbor)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ScopeSetRequest from a decoded CBOR value tree.
fn csil_dec_scope_set_request(csil_root: &CsilCborValue) -> Result<ScopeSetRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let grantee_cbor = {
        let csil_field = cbor_require(csil_root, "grantee_cbor")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(ScopeSetRequest {
        ctx,
        grantee_cbor,
        scopes,
    })
}

/// Encode a ScopeSetRequest to canonical CSIL CBOR bytes.
pub fn encode_scope_set_request(csil_v: &ScopeSetRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_scope_set_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ScopeSetRequest.
pub fn decode_scope_set_request(csil_data: &[u8]) -> Result<ScopeSetRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_scope_set_request(&csil_root)
}

/// Build the canonical CBOR value tree for a SignedActAsScopeSet.
fn csil_enc_signed_act_as_scope_set(csil_v: &SignedActAsScopeSet) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((
        cbor_text("scopes"),
        cbor_enc_array(&csil_v.scopes, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("expires_at"), cbor_int(csil_v.expires_at)));
    csil_entries.push((
        cbor_text("scope_set_cbor"),
        cbor_bytes(&csil_v.scope_set_cbor),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SignedActAsScopeSet from a decoded CBOR value tree.
fn csil_dec_signed_act_as_scope_set(
    csil_root: &CsilCborValue,
) -> Result<SignedActAsScopeSet, CsilCborError> {
    let scope_set_cbor = {
        let csil_field = cbor_require(csil_root, "scope_set_cbor")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let scopes = {
        let csil_field = cbor_require(csil_root, "scopes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let expires_at = {
        let csil_field = cbor_require(csil_root, "expires_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(SignedActAsScopeSet {
        scope_set_cbor,
        scopes,
        expires_at,
    })
}

/// Encode a SignedActAsScopeSet to canonical CSIL CBOR bytes.
pub fn encode_signed_act_as_scope_set(csil_v: &SignedActAsScopeSet) -> Vec<u8> {
    cbor_encode(&csil_enc_signed_act_as_scope_set(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SignedActAsScopeSet.
pub fn decode_signed_act_as_scope_set(
    csil_data: &[u8],
) -> Result<SignedActAsScopeSet, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_signed_act_as_scope_set(&csil_root)
}

/// Build the canonical CBOR value tree for a SshKeyInfo.
fn csil_enc_ssh_key_info(csil_v: &SshKeyInfo) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(8);
    csil_entries.push((cbor_text("usage"), cbor_text(&csil_v.usage)));
    csil_entries.push((cbor_text("key_id"), cbor_text(&csil_v.key_id)));
    csil_entries.push((cbor_text("status"), cbor_text(&csil_v.status)));
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    csil_entries.push((cbor_text("public_key"), cbor_text(&csil_v.public_key)));
    if let Some(csil_inner) = &csil_v.instance_id {
        csil_entries.push((cbor_text("instance_id"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.application_id {
        csil_entries.push((cbor_text("application_id"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.attested_until {
        csil_entries.push((cbor_text("attested_until"), cbor_int(*csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SshKeyInfo from a decoded CBOR value tree.
fn csil_dec_ssh_key_info(csil_root: &CsilCborValue) -> Result<SshKeyInfo, CsilCborError> {
    let key_id = {
        let csil_field = cbor_require(csil_root, "key_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let public_key = {
        let csil_field = cbor_require(csil_root, "public_key")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let usage = {
        let csil_field = cbor_require(csil_root, "usage")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let status = {
        let csil_field = cbor_require(csil_root, "status")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let application_id = match cbor_map_get(csil_root, "application_id") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let instance_id = match cbor_map_get(csil_root, "instance_id") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let attested_until = match cbor_map_get(csil_root, "attested_until") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(SshKeyInfo {
        key_id,
        public_key,
        usage,
        status,
        application_id,
        instance_id,
        attested_until,
        created_at,
    })
}

/// Encode a SshKeyInfo to canonical CSIL CBOR bytes.
pub fn encode_ssh_key_info(csil_v: &SshKeyInfo) -> Vec<u8> {
    cbor_encode(&csil_enc_ssh_key_info(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SshKeyInfo.
pub fn decode_ssh_key_info(csil_data: &[u8]) -> Result<SshKeyInfo, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_ssh_key_info(&csil_root)
}

/// Build the canonical CBOR value tree for a RegisterKeyRequest.
fn csil_enc_register_key_request(csil_v: &RegisterKeyRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("usage"), cbor_text(&csil_v.usage)));
    if let Some(csil_inner) = &csil_v.lk_key_id {
        csil_entries.push((cbor_text("lk_key_id"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("public_key"), cbor_text(&csil_v.public_key)));
    if let Some(csil_inner) = &csil_v.instance_id {
        csil_entries.push((cbor_text("instance_id"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.application_id {
        csil_entries.push((cbor_text("application_id"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RegisterKeyRequest from a decoded CBOR value tree.
fn csil_dec_register_key_request(
    csil_root: &CsilCborValue,
) -> Result<RegisterKeyRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let public_key = {
        let csil_field = cbor_require(csil_root, "public_key")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let usage = {
        let csil_field = cbor_require(csil_root, "usage")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let application_id = match cbor_map_get(csil_root, "application_id") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let instance_id = match cbor_map_get(csil_root, "instance_id") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let lk_key_id = match cbor_map_get(csil_root, "lk_key_id") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(RegisterKeyRequest {
        ctx,
        public_key,
        usage,
        application_id,
        instance_id,
        lk_key_id,
    })
}

/// Encode a RegisterKeyRequest to canonical CSIL CBOR bytes.
pub fn encode_register_key_request(csil_v: &RegisterKeyRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_register_key_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RegisterKeyRequest.
pub fn decode_register_key_request(csil_data: &[u8]) -> Result<RegisterKeyRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_register_key_request(&csil_root)
}

/// Build the canonical CBOR value tree for a KeyRef.
fn csil_enc_key_ref(csil_v: &KeyRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("key_id"), cbor_text(&csil_v.key_id)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a KeyRef from a decoded CBOR value tree.
fn csil_dec_key_ref(csil_root: &CsilCborValue) -> Result<KeyRef, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let key_id = {
        let csil_field = cbor_require(csil_root, "key_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(KeyRef { ctx, key_id })
}

/// Encode a KeyRef to canonical CSIL CBOR bytes.
pub fn encode_key_ref(csil_v: &KeyRef) -> Vec<u8> {
    cbor_encode(&csil_enc_key_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a KeyRef.
pub fn decode_key_ref(csil_data: &[u8]) -> Result<KeyRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_key_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a ListKeysResponse.
fn csil_enc_list_keys_response(csil_v: &ListKeysResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("keys"),
        cbor_enc_array(&csil_v.keys, csil_enc_ssh_key_info),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListKeysResponse from a decoded CBOR value tree.
fn csil_dec_list_keys_response(
    csil_root: &CsilCborValue,
) -> Result<ListKeysResponse, CsilCborError> {
    let keys = {
        let csil_field = cbor_require(csil_root, "keys")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_ssh_key_info);
        csil_decode(csil_field)?
    };
    Ok(ListKeysResponse { keys })
}

/// Encode a ListKeysResponse to canonical CSIL CBOR bytes.
pub fn encode_list_keys_response(csil_v: &ListKeysResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_keys_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListKeysResponse.
pub fn decode_list_keys_response(csil_data: &[u8]) -> Result<ListKeysResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_keys_response(&csil_root)
}

/// Build the canonical CBOR value tree for a Org.
fn csil_enc_org(csil_v: &Org) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    if let Some(csil_inner) = &csil_v.display_name {
        csil_entries.push((cbor_text("display_name"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.github_owner {
        csil_entries.push((cbor_text("github_owner"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Org from a decoded CBOR value tree.
fn csil_dec_org(csil_root: &CsilCborValue) -> Result<Org, CsilCborError> {
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let display_name = match cbor_map_get(csil_root, "display_name") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let github_owner = match cbor_map_get(csil_root, "github_owner") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(Org {
        name,
        display_name,
        github_owner,
        created_at,
    })
}

/// Encode a Org to canonical CSIL CBOR bytes.
pub fn encode_org(csil_v: &Org) -> Vec<u8> {
    cbor_encode(&csil_enc_org(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Org.
pub fn decode_org(csil_data: &[u8]) -> Result<Org, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_org(&csil_root)
}

/// Build the canonical CBOR value tree for a CreateOrgRequest.
fn csil_enc_create_org_request(csil_v: &CreateOrgRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.display_name {
        csil_entries.push((cbor_text("display_name"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.github_owner {
        csil_entries.push((cbor_text("github_owner"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CreateOrgRequest from a decoded CBOR value tree.
fn csil_dec_create_org_request(
    csil_root: &CsilCborValue,
) -> Result<CreateOrgRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let display_name = match cbor_map_get(csil_root, "display_name") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let github_owner = match cbor_map_get(csil_root, "github_owner") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CreateOrgRequest {
        ctx,
        name,
        display_name,
        github_owner,
    })
}

/// Encode a CreateOrgRequest to canonical CSIL CBOR bytes.
pub fn encode_create_org_request(csil_v: &CreateOrgRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_create_org_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CreateOrgRequest.
pub fn decode_create_org_request(csil_data: &[u8]) -> Result<CreateOrgRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_create_org_request(&csil_root)
}

/// Build the canonical CBOR value tree for a GetOrgRequest.
fn csil_enc_get_org_request(csil_v: &GetOrgRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a GetOrgRequest from a decoded CBOR value tree.
fn csil_dec_get_org_request(csil_root: &CsilCborValue) -> Result<GetOrgRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(GetOrgRequest { ctx, org })
}

/// Encode a GetOrgRequest to canonical CSIL CBOR bytes.
pub fn encode_get_org_request(csil_v: &GetOrgRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_get_org_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a GetOrgRequest.
pub fn decode_get_org_request(csil_data: &[u8]) -> Result<GetOrgRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_get_org_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListOrgsRequest.
fn csil_enc_list_orgs_request(csil_v: &ListOrgsRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListOrgsRequest from a decoded CBOR value tree.
fn csil_dec_list_orgs_request(csil_root: &CsilCborValue) -> Result<ListOrgsRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ListOrgsRequest { ctx, page })
}

/// Encode a ListOrgsRequest to canonical CSIL CBOR bytes.
pub fn encode_list_orgs_request(csil_v: &ListOrgsRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_orgs_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListOrgsRequest.
pub fn decode_list_orgs_request(csil_data: &[u8]) -> Result<ListOrgsRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_orgs_request(&csil_root)
}

/// Build the canonical CBOR value tree for a OrgPage.
fn csil_enc_org_page(csil_v: &OrgPage) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("orgs"),
        cbor_enc_array(&csil_v.orgs, csil_enc_org),
    ));
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a OrgPage from a decoded CBOR value tree.
fn csil_dec_org_page(csil_root: &CsilCborValue) -> Result<OrgPage, CsilCborError> {
    let orgs = {
        let csil_field = cbor_require(csil_root, "orgs")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_org);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(OrgPage { orgs, next_cursor })
}

/// Encode a OrgPage to canonical CSIL CBOR bytes.
pub fn encode_org_page(csil_v: &OrgPage) -> Vec<u8> {
    cbor_encode(&csil_enc_org_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a OrgPage.
pub fn decode_org_page(csil_data: &[u8]) -> Result<OrgPage, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_org_page(&csil_root)
}

/// Build the canonical CBOR value tree for a UpdateOrgRequest.
fn csil_enc_update_org_request(csil_v: &UpdateOrgRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.display_name {
        csil_entries.push((cbor_text("display_name"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.github_owner {
        csil_entries.push((cbor_text("github_owner"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UpdateOrgRequest from a decoded CBOR value tree.
fn csil_dec_update_org_request(
    csil_root: &CsilCborValue,
) -> Result<UpdateOrgRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let display_name = match cbor_map_get(csil_root, "display_name") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let github_owner = match cbor_map_get(csil_root, "github_owner") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(UpdateOrgRequest {
        ctx,
        org,
        display_name,
        github_owner,
    })
}

/// Encode a UpdateOrgRequest to canonical CSIL CBOR bytes.
pub fn encode_update_org_request(csil_v: &UpdateOrgRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_update_org_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UpdateOrgRequest.
pub fn decode_update_org_request(csil_data: &[u8]) -> Result<UpdateOrgRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_update_org_request(&csil_root)
}

/// Build the canonical CBOR value tree for a OrgMember.
fn csil_enc_org_member(csil_v: &OrgMember) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("role"), cbor_text(&csil_v.role)));
    csil_entries.push((cbor_text("identity"), csil_enc_identity(&csil_v.identity)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a OrgMember from a decoded CBOR value tree.
fn csil_dec_org_member(csil_root: &CsilCborValue) -> Result<OrgMember, CsilCborError> {
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let role = {
        let csil_field = cbor_require(csil_root, "role")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(OrgMember { identity, role })
}

/// Encode a OrgMember to canonical CSIL CBOR bytes.
pub fn encode_org_member(csil_v: &OrgMember) -> Vec<u8> {
    cbor_encode(&csil_enc_org_member(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a OrgMember.
pub fn decode_org_member(csil_data: &[u8]) -> Result<OrgMember, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_org_member(&csil_root)
}

/// Build the canonical CBOR value tree for a SetOrgMemberRequest.
fn csil_enc_set_org_member_request(csil_v: &SetOrgMemberRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("role"), cbor_text(&csil_v.role)));
    csil_entries.push((cbor_text("identity"), cbor_text(&csil_v.identity)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SetOrgMemberRequest from a decoded CBOR value tree.
fn csil_dec_set_org_member_request(
    csil_root: &CsilCborValue,
) -> Result<SetOrgMemberRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let role = {
        let csil_field = cbor_require(csil_root, "role")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(SetOrgMemberRequest {
        ctx,
        org,
        identity,
        role,
    })
}

/// Encode a SetOrgMemberRequest to canonical CSIL CBOR bytes.
pub fn encode_set_org_member_request(csil_v: &SetOrgMemberRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_set_org_member_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SetOrgMemberRequest.
pub fn decode_set_org_member_request(
    csil_data: &[u8],
) -> Result<SetOrgMemberRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_set_org_member_request(&csil_root)
}

/// Build the canonical CBOR value tree for a RemoveOrgMemberRequest.
fn csil_enc_remove_org_member_request(csil_v: &RemoveOrgMemberRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("identity"), cbor_text(&csil_v.identity)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RemoveOrgMemberRequest from a decoded CBOR value tree.
fn csil_dec_remove_org_member_request(
    csil_root: &CsilCborValue,
) -> Result<RemoveOrgMemberRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(RemoveOrgMemberRequest { ctx, org, identity })
}

/// Encode a RemoveOrgMemberRequest to canonical CSIL CBOR bytes.
pub fn encode_remove_org_member_request(csil_v: &RemoveOrgMemberRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_remove_org_member_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RemoveOrgMemberRequest.
pub fn decode_remove_org_member_request(
    csil_data: &[u8],
) -> Result<RemoveOrgMemberRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_remove_org_member_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListOrgMembersRequest.
fn csil_enc_list_org_members_request(csil_v: &ListOrgMembersRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListOrgMembersRequest from a decoded CBOR value tree.
fn csil_dec_list_org_members_request(
    csil_root: &CsilCborValue,
) -> Result<ListOrgMembersRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ListOrgMembersRequest { ctx, org, page })
}

/// Encode a ListOrgMembersRequest to canonical CSIL CBOR bytes.
pub fn encode_list_org_members_request(csil_v: &ListOrgMembersRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_org_members_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListOrgMembersRequest.
pub fn decode_list_org_members_request(
    csil_data: &[u8],
) -> Result<ListOrgMembersRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_org_members_request(&csil_root)
}

/// Build the canonical CBOR value tree for a OrgMemberPage.
fn csil_enc_org_member_page(csil_v: &OrgMemberPage) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("members"),
        cbor_enc_array(&csil_v.members, csil_enc_org_member),
    ));
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a OrgMemberPage from a decoded CBOR value tree.
fn csil_dec_org_member_page(csil_root: &CsilCborValue) -> Result<OrgMemberPage, CsilCborError> {
    let members = {
        let csil_field = cbor_require(csil_root, "members")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_org_member);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(OrgMemberPage {
        members,
        next_cursor,
    })
}

/// Encode a OrgMemberPage to canonical CSIL CBOR bytes.
pub fn encode_org_member_page(csil_v: &OrgMemberPage) -> Vec<u8> {
    cbor_encode(&csil_enc_org_member_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a OrgMemberPage.
pub fn decode_org_member_page(csil_data: &[u8]) -> Result<OrgMemberPage, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_org_member_page(&csil_root)
}

/// Build the canonical CBOR value tree for a Team.
fn csil_enc_team(csil_v: &Team) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    csil_entries.push((
        cbor_text("members"),
        cbor_enc_array(&csil_v.members, csil_enc_identity),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Team from a decoded CBOR value tree.
fn csil_dec_team(csil_root: &CsilCborValue) -> Result<Team, CsilCborError> {
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let members = {
        let csil_field = cbor_require(csil_root, "members")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_identity);
        csil_decode(csil_field)?
    };
    Ok(Team { org, name, members })
}

/// Encode a Team to canonical CSIL CBOR bytes.
pub fn encode_team(csil_v: &Team) -> Vec<u8> {
    cbor_encode(&csil_enc_team(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Team.
pub fn decode_team(csil_data: &[u8]) -> Result<Team, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_team(&csil_root)
}

/// Build the canonical CBOR value tree for a CreateTeamRequest.
fn csil_enc_create_team_request(csil_v: &CreateTeamRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CreateTeamRequest from a decoded CBOR value tree.
fn csil_dec_create_team_request(
    csil_root: &CsilCborValue,
) -> Result<CreateTeamRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(CreateTeamRequest { ctx, org, name })
}

/// Encode a CreateTeamRequest to canonical CSIL CBOR bytes.
pub fn encode_create_team_request(csil_v: &CreateTeamRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_create_team_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CreateTeamRequest.
pub fn decode_create_team_request(csil_data: &[u8]) -> Result<CreateTeamRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_create_team_request(&csil_root)
}

/// Build the canonical CBOR value tree for a TeamRef.
fn csil_enc_team_ref(csil_v: &TeamRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("team"), cbor_text(&csil_v.team)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a TeamRef from a decoded CBOR value tree.
fn csil_dec_team_ref(csil_root: &CsilCborValue) -> Result<TeamRef, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let team = {
        let csil_field = cbor_require(csil_root, "team")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(TeamRef { ctx, org, team })
}

/// Encode a TeamRef to canonical CSIL CBOR bytes.
pub fn encode_team_ref(csil_v: &TeamRef) -> Vec<u8> {
    cbor_encode(&csil_enc_team_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a TeamRef.
pub fn decode_team_ref(csil_data: &[u8]) -> Result<TeamRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_team_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a SetTeamMemberRequest.
fn csil_enc_set_team_member_request(csil_v: &SetTeamMemberRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("team"), cbor_text(&csil_v.team)));
    csil_entries.push((cbor_text("member"), cbor_bool(csil_v.member)));
    csil_entries.push((cbor_text("identity"), cbor_text(&csil_v.identity)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SetTeamMemberRequest from a decoded CBOR value tree.
fn csil_dec_set_team_member_request(
    csil_root: &CsilCborValue,
) -> Result<SetTeamMemberRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let team = {
        let csil_field = cbor_require(csil_root, "team")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let member = {
        let csil_field = cbor_require(csil_root, "member")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(SetTeamMemberRequest {
        ctx,
        org,
        team,
        identity,
        member,
    })
}

/// Encode a SetTeamMemberRequest to canonical CSIL CBOR bytes.
pub fn encode_set_team_member_request(csil_v: &SetTeamMemberRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_set_team_member_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SetTeamMemberRequest.
pub fn decode_set_team_member_request(
    csil_data: &[u8],
) -> Result<SetTeamMemberRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_set_team_member_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListTeamsResponse.
fn csil_enc_list_teams_response(csil_v: &ListTeamsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("teams"),
        cbor_enc_array(&csil_v.teams, csil_enc_team),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListTeamsResponse from a decoded CBOR value tree.
fn csil_dec_list_teams_response(
    csil_root: &CsilCborValue,
) -> Result<ListTeamsResponse, CsilCborError> {
    let teams = {
        let csil_field = cbor_require(csil_root, "teams")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_team);
        csil_decode(csil_field)?
    };
    Ok(ListTeamsResponse { teams })
}

/// Encode a ListTeamsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_teams_response(csil_v: &ListTeamsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_teams_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListTeamsResponse.
pub fn decode_list_teams_response(csil_data: &[u8]) -> Result<ListTeamsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_teams_response(&csil_root)
}

/// Build the canonical CBOR value tree for a Repo.
fn csil_enc_repo(csil_v: &Repo) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(12);
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.fork_of {
        csil_entries.push((cbor_text("fork_of"), csil_enc_repo_ref(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.ssh_url {
        csil_entries.push((cbor_text("ssh_url"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("archived"), cbor_bool(csil_v.archived)));
    csil_entries.push((cbor_text("vcs_type"), cbor_text(&csil_v.vcs_type)));
    csil_entries.push((cbor_text("https_url"), cbor_text(&csil_v.https_url)));
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    csil_entries.push((cbor_text("visibility"), cbor_text(&csil_v.visibility)));
    if let Some(csil_inner) = &csil_v.description {
        csil_entries.push((cbor_text("description"), cbor_text(csil_inner)));
    }
    csil_entries.push((
        cbor_text("default_branch"),
        cbor_text(&csil_v.default_branch),
    ));
    csil_entries.push((
        cbor_text("mirror_enabled"),
        cbor_bool(csil_v.mirror_enabled),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Repo from a decoded CBOR value tree.
fn csil_dec_repo(csil_root: &CsilCborValue) -> Result<Repo, CsilCborError> {
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let vcs_type = {
        let csil_field = cbor_require(csil_root, "vcs_type")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let visibility = {
        let csil_field = cbor_require(csil_root, "visibility")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let default_branch = {
        let csil_field = cbor_require(csil_root, "default_branch")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let description = match cbor_map_get(csil_root, "description") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let fork_of = match cbor_map_get(csil_root, "fork_of") {
        Some(csil_field) => {
            let csil_decode = csil_dec_repo_ref;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let mirror_enabled = {
        let csil_field = cbor_require(csil_root, "mirror_enabled")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let archived = {
        let csil_field = cbor_require(csil_root, "archived")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let https_url = {
        let csil_field = cbor_require(csil_root, "https_url")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let ssh_url = match cbor_map_get(csil_root, "ssh_url") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Repo {
        org,
        name,
        vcs_type,
        visibility,
        default_branch,
        description,
        fork_of,
        mirror_enabled,
        archived,
        created_at,
        https_url,
        ssh_url,
    })
}

/// Encode a Repo to canonical CSIL CBOR bytes.
pub fn encode_repo(csil_v: &Repo) -> Vec<u8> {
    cbor_encode(&csil_enc_repo(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Repo.
pub fn decode_repo(csil_data: &[u8]) -> Result<Repo, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_repo(&csil_root)
}

/// Build the canonical CBOR value tree for a CreateRepoRequest.
fn csil_enc_create_repo_request(csil_v: &CreateRepoRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.vcs_type {
        csil_entries.push((cbor_text("vcs_type"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.visibility {
        csil_entries.push((cbor_text("visibility"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.description {
        csil_entries.push((cbor_text("description"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.default_branch {
        csil_entries.push((cbor_text("default_branch"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CreateRepoRequest from a decoded CBOR value tree.
fn csil_dec_create_repo_request(
    csil_root: &CsilCborValue,
) -> Result<CreateRepoRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let vcs_type = match cbor_map_get(csil_root, "vcs_type") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let visibility = match cbor_map_get(csil_root, "visibility") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let default_branch = match cbor_map_get(csil_root, "default_branch") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let description = match cbor_map_get(csil_root, "description") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CreateRepoRequest {
        ctx,
        org,
        name,
        vcs_type,
        visibility,
        default_branch,
        description,
    })
}

/// Encode a CreateRepoRequest to canonical CSIL CBOR bytes.
pub fn encode_create_repo_request(csil_v: &CreateRepoRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_create_repo_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CreateRepoRequest.
pub fn decode_create_repo_request(csil_data: &[u8]) -> Result<CreateRepoRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_create_repo_request(&csil_root)
}

/// Build the canonical CBOR value tree for a GetRepoRequest.
fn csil_enc_get_repo_request(csil_v: &GetRepoRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a GetRepoRequest from a decoded CBOR value tree.
fn csil_dec_get_repo_request(csil_root: &CsilCborValue) -> Result<GetRepoRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    Ok(GetRepoRequest { ctx, repo })
}

/// Encode a GetRepoRequest to canonical CSIL CBOR bytes.
pub fn encode_get_repo_request(csil_v: &GetRepoRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_get_repo_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a GetRepoRequest.
pub fn decode_get_repo_request(csil_data: &[u8]) -> Result<GetRepoRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_get_repo_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListReposRequest.
fn csil_enc_list_repos_request(csil_v: &ListReposRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.org {
        csil_entries.push((cbor_text("org"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListReposRequest from a decoded CBOR value tree.
fn csil_dec_list_repos_request(
    csil_root: &CsilCborValue,
) -> Result<ListReposRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = match cbor_map_get(csil_root, "org") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ListReposRequest { ctx, org, page })
}

/// Encode a ListReposRequest to canonical CSIL CBOR bytes.
pub fn encode_list_repos_request(csil_v: &ListReposRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_repos_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListReposRequest.
pub fn decode_list_repos_request(csil_data: &[u8]) -> Result<ListReposRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_repos_request(&csil_root)
}

/// Build the canonical CBOR value tree for a RepoPage.
fn csil_enc_repo_page(csil_v: &RepoPage) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("repos"),
        cbor_enc_array(&csil_v.repos, csil_enc_repo),
    ));
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RepoPage from a decoded CBOR value tree.
fn csil_dec_repo_page(csil_root: &CsilCborValue) -> Result<RepoPage, CsilCborError> {
    let repos = {
        let csil_field = cbor_require(csil_root, "repos")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_repo);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(RepoPage { repos, next_cursor })
}

/// Encode a RepoPage to canonical CSIL CBOR bytes.
pub fn encode_repo_page(csil_v: &RepoPage) -> Vec<u8> {
    cbor_encode(&csil_enc_repo_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RepoPage.
pub fn decode_repo_page(csil_data: &[u8]) -> Result<RepoPage, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_repo_page(&csil_root)
}

/// Build the canonical CBOR value tree for a UpdateRepoRequest.
fn csil_enc_update_repo_request(csil_v: &UpdateRepoRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.archived {
        csil_entries.push((cbor_text("archived"), cbor_bool(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.new_name {
        csil_entries.push((cbor_text("new_name"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.visibility {
        csil_entries.push((cbor_text("visibility"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.description {
        csil_entries.push((cbor_text("description"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.default_branch {
        csil_entries.push((cbor_text("default_branch"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UpdateRepoRequest from a decoded CBOR value tree.
fn csil_dec_update_repo_request(
    csil_root: &CsilCborValue,
) -> Result<UpdateRepoRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let description = match cbor_map_get(csil_root, "description") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let default_branch = match cbor_map_get(csil_root, "default_branch") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let visibility = match cbor_map_get(csil_root, "visibility") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let archived = match cbor_map_get(csil_root, "archived") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bool;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let new_name = match cbor_map_get(csil_root, "new_name") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(UpdateRepoRequest {
        ctx,
        repo,
        description,
        default_branch,
        visibility,
        archived,
        new_name,
    })
}

/// Encode a UpdateRepoRequest to canonical CSIL CBOR bytes.
pub fn encode_update_repo_request(csil_v: &UpdateRepoRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_update_repo_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UpdateRepoRequest.
pub fn decode_update_repo_request(csil_data: &[u8]) -> Result<UpdateRepoRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_update_repo_request(&csil_root)
}

/// Build the canonical CBOR value tree for a DeleteRepoRequest.
fn csil_enc_delete_repo_request(csil_v: &DeleteRepoRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("confirm_name"), cbor_text(&csil_v.confirm_name)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DeleteRepoRequest from a decoded CBOR value tree.
fn csil_dec_delete_repo_request(
    csil_root: &CsilCborValue,
) -> Result<DeleteRepoRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let confirm_name = {
        let csil_field = cbor_require(csil_root, "confirm_name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(DeleteRepoRequest {
        ctx,
        repo,
        confirm_name,
    })
}

/// Encode a DeleteRepoRequest to canonical CSIL CBOR bytes.
pub fn encode_delete_repo_request(csil_v: &DeleteRepoRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_delete_repo_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DeleteRepoRequest.
pub fn decode_delete_repo_request(csil_data: &[u8]) -> Result<DeleteRepoRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_delete_repo_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ForkRequest.
fn csil_enc_fork_request(csil_v: &ForkRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.name {
        csil_entries.push((cbor_text("name"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("source"), csil_enc_repo_ref(&csil_v.source)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ForkRequest from a decoded CBOR value tree.
fn csil_dec_fork_request(csil_root: &CsilCborValue) -> Result<ForkRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let source = {
        let csil_field = cbor_require(csil_root, "source")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = match cbor_map_get(csil_root, "name") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ForkRequest {
        ctx,
        source,
        org,
        name,
    })
}

/// Encode a ForkRequest to canonical CSIL CBOR bytes.
pub fn encode_fork_request(csil_v: &ForkRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_fork_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ForkRequest.
pub fn decode_fork_request(csil_data: &[u8]) -> Result<ForkRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_fork_request(&csil_root)
}

/// Build the canonical CBOR value tree for a RefInfo.
fn csil_enc_ref_info(csil_v: &RefInfo) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.peeled {
        csil_entries.push((cbor_text("peeled"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("target"), cbor_text(&csil_v.target)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RefInfo from a decoded CBOR value tree.
fn csil_dec_ref_info(csil_root: &CsilCborValue) -> Result<RefInfo, CsilCborError> {
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let target = {
        let csil_field = cbor_require(csil_root, "target")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let peeled = match cbor_map_get(csil_root, "peeled") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(RefInfo {
        name,
        target,
        peeled,
    })
}

/// Encode a RefInfo to canonical CSIL CBOR bytes.
pub fn encode_ref_info(csil_v: &RefInfo) -> Vec<u8> {
    cbor_encode(&csil_enc_ref_info(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RefInfo.
pub fn decode_ref_info(csil_data: &[u8]) -> Result<RefInfo, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_ref_info(&csil_root)
}

/// Build the canonical CBOR value tree for a ListRefsRequest.
fn csil_enc_list_refs_request(csil_v: &ListRefsRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.prefix {
        csil_entries.push((cbor_text("prefix"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListRefsRequest from a decoded CBOR value tree.
fn csil_dec_list_refs_request(csil_root: &CsilCborValue) -> Result<ListRefsRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let prefix = match cbor_map_get(csil_root, "prefix") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ListRefsRequest { ctx, repo, prefix })
}

/// Encode a ListRefsRequest to canonical CSIL CBOR bytes.
pub fn encode_list_refs_request(csil_v: &ListRefsRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_refs_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListRefsRequest.
pub fn decode_list_refs_request(csil_data: &[u8]) -> Result<ListRefsRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_refs_request(&csil_root)
}

/// Build the canonical CBOR value tree for a CaptureReport.
fn csil_enc_capture_report(csil_v: &CaptureReport) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("bytes"), cbor_uint(csil_v.bytes)));
    csil_entries.push((cbor_text("objects"), cbor_uint(csil_v.objects)));
    if let Some(csil_inner) = &csil_v.segment {
        csil_entries.push((cbor_text("segment"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("uploaded"), cbor_bool(csil_v.uploaded)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CaptureReport from a decoded CBOR value tree.
fn csil_dec_capture_report(csil_root: &CsilCborValue) -> Result<CaptureReport, CsilCborError> {
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let uploaded = {
        let csil_field = cbor_require(csil_root, "uploaded")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let objects = {
        let csil_field = cbor_require(csil_root, "objects")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let bytes = {
        let csil_field = cbor_require(csil_root, "bytes")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let segment = match cbor_map_get(csil_root, "segment") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CaptureReport {
        repo,
        uploaded,
        objects,
        bytes,
        segment,
    })
}

/// Encode a CaptureReport to canonical CSIL CBOR bytes.
pub fn encode_capture_report(csil_v: &CaptureReport) -> Vec<u8> {
    cbor_encode(&csil_enc_capture_report(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CaptureReport.
pub fn decode_capture_report(csil_data: &[u8]) -> Result<CaptureReport, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_capture_report(&csil_root)
}

/// Build the canonical CBOR value tree for a ListRefsResponse.
fn csil_enc_list_refs_response(csil_v: &ListRefsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("refs"),
        cbor_enc_array(&csil_v.refs, csil_enc_ref_info),
    ));
    csil_entries.push((cbor_text("refs_version"), cbor_uint(csil_v.refs_version)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListRefsResponse from a decoded CBOR value tree.
fn csil_dec_list_refs_response(
    csil_root: &CsilCborValue,
) -> Result<ListRefsResponse, CsilCborError> {
    let refs = {
        let csil_field = cbor_require(csil_root, "refs")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_ref_info);
        csil_decode(csil_field)?
    };
    let refs_version = {
        let csil_field = cbor_require(csil_root, "refs_version")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    Ok(ListRefsResponse { refs, refs_version })
}

/// Encode a ListRefsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_refs_response(csil_v: &ListRefsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_refs_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListRefsResponse.
pub fn decode_list_refs_response(csil_data: &[u8]) -> Result<ListRefsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_refs_response(&csil_root)
}

/// Build the canonical CBOR value tree for a UpdateRefRequest.
fn csil_enc_update_ref_request(csil_v: &UpdateRefRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.target {
        csil_entries.push((cbor_text("target"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.expected {
        csil_entries.push((cbor_text("expected"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UpdateRefRequest from a decoded CBOR value tree.
fn csil_dec_update_ref_request(
    csil_root: &CsilCborValue,
) -> Result<UpdateRefRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let expected = match cbor_map_get(csil_root, "expected") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let target = match cbor_map_get(csil_root, "target") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(UpdateRefRequest {
        ctx,
        repo,
        name,
        expected,
        target,
    })
}

/// Encode a UpdateRefRequest to canonical CSIL CBOR bytes.
pub fn encode_update_ref_request(csil_v: &UpdateRefRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_update_ref_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UpdateRefRequest.
pub fn decode_update_ref_request(csil_data: &[u8]) -> Result<UpdateRefRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_update_ref_request(&csil_root)
}

/// Build the canonical CBOR value tree for a Signature.
fn csil_enc_signature(csil_v: &Signature) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    if let Some(csil_inner) = &csil_v.signer {
        csil_entries.push((cbor_text("signer"), csil_enc_identity(csil_inner)));
    }
    csil_entries.push((cbor_text("status"), cbor_text(&csil_v.status)));
    if let Some(csil_inner) = &csil_v.key_fingerprint {
        csil_entries.push((cbor_text("key_fingerprint"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Signature from a decoded CBOR value tree.
fn csil_dec_signature(csil_root: &CsilCborValue) -> Result<Signature, CsilCborError> {
    let status = {
        let csil_field = cbor_require(csil_root, "status")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let signer = match cbor_map_get(csil_root, "signer") {
        Some(csil_field) => {
            let csil_decode = csil_dec_identity;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let key_fingerprint = match cbor_map_get(csil_root, "key_fingerprint") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Signature {
        status,
        signer,
        key_fingerprint,
    })
}

/// Encode a Signature to canonical CSIL CBOR bytes.
pub fn encode_signature(csil_v: &Signature) -> Vec<u8> {
    cbor_encode(&csil_enc_signature(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Signature.
pub fn decode_signature(csil_data: &[u8]) -> Result<Signature, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_signature(&csil_root)
}

/// Build the canonical CBOR value tree for a Commit.
fn csil_enc_commit(csil_v: &Commit) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(11);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("tree"), cbor_text(&csil_v.tree)));
    csil_entries.push((cbor_text("message"), cbor_text(&csil_v.message)));
    csil_entries.push((
        cbor_text("parents"),
        cbor_enc_array(&csil_v.parents, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((
        cbor_text("signature"),
        csil_enc_signature(&csil_v.signature),
    ));
    csil_entries.push((cbor_text("author_name"), cbor_text(&csil_v.author_name)));
    csil_entries.push((cbor_text("authored_at"), cbor_int(csil_v.authored_at)));
    csil_entries.push((cbor_text("author_email"), cbor_text(&csil_v.author_email)));
    csil_entries.push((cbor_text("committed_at"), cbor_int(csil_v.committed_at)));
    csil_entries.push((
        cbor_text("committer_name"),
        cbor_text(&csil_v.committer_name),
    ));
    csil_entries.push((
        cbor_text("committer_email"),
        cbor_text(&csil_v.committer_email),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Commit from a decoded CBOR value tree.
fn csil_dec_commit(csil_root: &CsilCborValue) -> Result<Commit, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let parents = {
        let csil_field = cbor_require(csil_root, "parents")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let tree = {
        let csil_field = cbor_require(csil_root, "tree")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let author_name = {
        let csil_field = cbor_require(csil_root, "author_name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let author_email = {
        let csil_field = cbor_require(csil_root, "author_email")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let authored_at = {
        let csil_field = cbor_require(csil_root, "authored_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let committer_name = {
        let csil_field = cbor_require(csil_root, "committer_name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let committer_email = {
        let csil_field = cbor_require(csil_root, "committer_email")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let committed_at = {
        let csil_field = cbor_require(csil_root, "committed_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let message = {
        let csil_field = cbor_require(csil_root, "message")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let signature = {
        let csil_field = cbor_require(csil_root, "signature")?;
        let csil_decode = csil_dec_signature;
        csil_decode(csil_field)?
    };
    Ok(Commit {
        id,
        parents,
        tree,
        author_name,
        author_email,
        authored_at,
        committer_name,
        committer_email,
        committed_at,
        message,
        signature,
    })
}

/// Encode a Commit to canonical CSIL CBOR bytes.
pub fn encode_commit(csil_v: &Commit) -> Vec<u8> {
    cbor_encode(&csil_enc_commit(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Commit.
pub fn decode_commit(csil_data: &[u8]) -> Result<Commit, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_commit(&csil_root)
}

/// Build the canonical CBOR value tree for a LogRequest.
fn csil_enc_log_request(csil_v: &LogRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("rev"), cbor_text(&csil_v.rev)));
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.path {
        csil_entries.push((cbor_text("path"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a LogRequest from a decoded CBOR value tree.
fn csil_dec_log_request(csil_root: &CsilCborValue) -> Result<LogRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let rev = {
        let csil_field = cbor_require(csil_root, "rev")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let path = match cbor_map_get(csil_root, "path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(LogRequest {
        ctx,
        repo,
        rev,
        path,
        page,
    })
}

/// Encode a LogRequest to canonical CSIL CBOR bytes.
pub fn encode_log_request(csil_v: &LogRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_log_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a LogRequest.
pub fn decode_log_request(csil_data: &[u8]) -> Result<LogRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_log_request(&csil_root)
}

/// Build the canonical CBOR value tree for a CommitPage.
fn csil_enc_commit_page(csil_v: &CommitPage) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("commits"),
        cbor_enc_array(&csil_v.commits, csil_enc_commit),
    ));
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CommitPage from a decoded CBOR value tree.
fn csil_dec_commit_page(csil_root: &CsilCborValue) -> Result<CommitPage, CsilCborError> {
    let commits = {
        let csil_field = cbor_require(csil_root, "commits")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_commit);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CommitPage {
        commits,
        next_cursor,
    })
}

/// Encode a CommitPage to canonical CSIL CBOR bytes.
pub fn encode_commit_page(csil_v: &CommitPage) -> Vec<u8> {
    cbor_encode(&csil_enc_commit_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CommitPage.
pub fn decode_commit_page(csil_data: &[u8]) -> Result<CommitPage, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_commit_page(&csil_root)
}

/// Build the canonical CBOR value tree for a GetCommitRequest.
fn csil_enc_get_commit_request(csil_v: &GetCommitRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("rev"), cbor_text(&csil_v.rev)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a GetCommitRequest from a decoded CBOR value tree.
fn csil_dec_get_commit_request(
    csil_root: &CsilCborValue,
) -> Result<GetCommitRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let rev = {
        let csil_field = cbor_require(csil_root, "rev")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(GetCommitRequest { ctx, repo, rev })
}

/// Encode a GetCommitRequest to canonical CSIL CBOR bytes.
pub fn encode_get_commit_request(csil_v: &GetCommitRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_get_commit_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a GetCommitRequest.
pub fn decode_get_commit_request(csil_data: &[u8]) -> Result<GetCommitRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_get_commit_request(&csil_root)
}

/// Build the canonical CBOR value tree for a TreeEntry.
fn csil_enc_tree_entry(csil_v: &TreeEntry) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("kind"), cbor_text(&csil_v.kind)));
    csil_entries.push((cbor_text("mode"), cbor_uint(csil_v.mode)));
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.size {
        csil_entries.push((cbor_text("size"), cbor_uint(*csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a TreeEntry from a decoded CBOR value tree.
fn csil_dec_tree_entry(csil_root: &CsilCborValue) -> Result<TreeEntry, CsilCborError> {
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let kind = {
        let csil_field = cbor_require(csil_root, "kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let mode = {
        let csil_field = cbor_require(csil_root, "mode")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let size = match cbor_map_get(csil_root, "size") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(TreeEntry {
        name,
        kind,
        id,
        mode,
        size,
    })
}

/// Encode a TreeEntry to canonical CSIL CBOR bytes.
pub fn encode_tree_entry(csil_v: &TreeEntry) -> Vec<u8> {
    cbor_encode(&csil_enc_tree_entry(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a TreeEntry.
pub fn decode_tree_entry(csil_data: &[u8]) -> Result<TreeEntry, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_tree_entry(&csil_root)
}

/// Build the canonical CBOR value tree for a TreeRequest.
fn csil_enc_tree_request(csil_v: &TreeRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("rev"), cbor_text(&csil_v.rev)));
    if let Some(csil_inner) = &csil_v.path {
        csil_entries.push((cbor_text("path"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a TreeRequest from a decoded CBOR value tree.
fn csil_dec_tree_request(csil_root: &CsilCborValue) -> Result<TreeRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let rev = {
        let csil_field = cbor_require(csil_root, "rev")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let path = match cbor_map_get(csil_root, "path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(TreeRequest {
        ctx,
        repo,
        rev,
        path,
    })
}

/// Encode a TreeRequest to canonical CSIL CBOR bytes.
pub fn encode_tree_request(csil_v: &TreeRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_tree_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a TreeRequest.
pub fn decode_tree_request(csil_data: &[u8]) -> Result<TreeRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_tree_request(&csil_root)
}

/// Build the canonical CBOR value tree for a TreeResponse.
fn csil_enc_tree_response(csil_v: &TreeResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("rev_id"), cbor_text(&csil_v.rev_id)));
    csil_entries.push((
        cbor_text("entries"),
        cbor_enc_array(&csil_v.entries, csil_enc_tree_entry),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a TreeResponse from a decoded CBOR value tree.
fn csil_dec_tree_response(csil_root: &CsilCborValue) -> Result<TreeResponse, CsilCborError> {
    let entries = {
        let csil_field = cbor_require(csil_root, "entries")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_tree_entry);
        csil_decode(csil_field)?
    };
    let rev_id = {
        let csil_field = cbor_require(csil_root, "rev_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(TreeResponse { entries, rev_id })
}

/// Encode a TreeResponse to canonical CSIL CBOR bytes.
pub fn encode_tree_response(csil_v: &TreeResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_tree_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a TreeResponse.
pub fn decode_tree_response(csil_data: &[u8]) -> Result<TreeResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_tree_response(&csil_root)
}

/// Build the canonical CBOR value tree for a BlobRequest.
fn csil_enc_blob_request(csil_v: &BlobRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("rev"), cbor_text(&csil_v.rev)));
    csil_entries.push((cbor_text("path"), cbor_text(&csil_v.path)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.limit {
        csil_entries.push((cbor_text("limit"), cbor_uint(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.offset {
        csil_entries.push((cbor_text("offset"), cbor_uint(*csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BlobRequest from a decoded CBOR value tree.
fn csil_dec_blob_request(csil_root: &CsilCborValue) -> Result<BlobRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let rev = {
        let csil_field = cbor_require(csil_root, "rev")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let path = {
        let csil_field = cbor_require(csil_root, "path")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let offset = match cbor_map_get(csil_root, "offset") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let limit = match cbor_map_get(csil_root, "limit") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(BlobRequest {
        ctx,
        repo,
        rev,
        path,
        offset,
        limit,
    })
}

/// Encode a BlobRequest to canonical CSIL CBOR bytes.
pub fn encode_blob_request(csil_v: &BlobRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_blob_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BlobRequest.
pub fn decode_blob_request(csil_data: &[u8]) -> Result<BlobRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_blob_request(&csil_root)
}

/// Build the canonical CBOR value tree for a BlobResponse.
fn csil_enc_blob_response(csil_v: &BlobResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("size"), cbor_uint(csil_v.size)));
    csil_entries.push((cbor_text("binary"), cbor_bool(csil_v.binary)));
    csil_entries.push((cbor_text("offset"), cbor_uint(csil_v.offset)));
    csil_entries.push((cbor_text("content"), cbor_bytes(&csil_v.content)));
    csil_entries.push((cbor_text("truncated"), cbor_bool(csil_v.truncated)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BlobResponse from a decoded CBOR value tree.
fn csil_dec_blob_response(csil_root: &CsilCborValue) -> Result<BlobResponse, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let size = {
        let csil_field = cbor_require(csil_root, "size")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let offset = {
        let csil_field = cbor_require(csil_root, "offset")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let content = {
        let csil_field = cbor_require(csil_root, "content")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let truncated = {
        let csil_field = cbor_require(csil_root, "truncated")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let binary = {
        let csil_field = cbor_require(csil_root, "binary")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(BlobResponse {
        id,
        size,
        offset,
        content,
        truncated,
        binary,
    })
}

/// Encode a BlobResponse to canonical CSIL CBOR bytes.
pub fn encode_blob_response(csil_v: &BlobResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_blob_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BlobResponse.
pub fn decode_blob_response(csil_data: &[u8]) -> Result<BlobResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_blob_response(&csil_root)
}

/// Build the canonical CBOR value tree for a DiffRequest.
fn csil_enc_diff_request(csil_v: &DiffRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("base"), cbor_text(&csil_v.base)));
    csil_entries.push((cbor_text("head"), cbor_text(&csil_v.head)));
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.path {
        csil_entries.push((cbor_text("path"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DiffRequest from a decoded CBOR value tree.
fn csil_dec_diff_request(csil_root: &CsilCborValue) -> Result<DiffRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let base = {
        let csil_field = cbor_require(csil_root, "base")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let head = {
        let csil_field = cbor_require(csil_root, "head")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let path = match cbor_map_get(csil_root, "path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(DiffRequest {
        ctx,
        repo,
        base,
        head,
        path,
        page,
    })
}

/// Encode a DiffRequest to canonical CSIL CBOR bytes.
pub fn encode_diff_request(csil_v: &DiffRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_diff_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DiffRequest.
pub fn decode_diff_request(csil_data: &[u8]) -> Result<DiffRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_diff_request(&csil_root)
}

/// Build the canonical CBOR value tree for a FileDiff.
fn csil_enc_file_diff(csil_v: &FileDiff) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    if let Some(csil_inner) = &csil_v.patch {
        csil_entries.push((cbor_text("patch"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("status"), cbor_text(&csil_v.status)));
    if let Some(csil_inner) = &csil_v.new_path {
        csil_entries.push((cbor_text("new_path"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.old_path {
        csil_entries.push((cbor_text("old_path"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("additions"), cbor_uint(csil_v.additions)));
    csil_entries.push((cbor_text("deletions"), cbor_uint(csil_v.deletions)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a FileDiff from a decoded CBOR value tree.
fn csil_dec_file_diff(csil_root: &CsilCborValue) -> Result<FileDiff, CsilCborError> {
    let status = {
        let csil_field = cbor_require(csil_root, "status")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let old_path = match cbor_map_get(csil_root, "old_path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let new_path = match cbor_map_get(csil_root, "new_path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let additions = {
        let csil_field = cbor_require(csil_root, "additions")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let deletions = {
        let csil_field = cbor_require(csil_root, "deletions")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let patch = match cbor_map_get(csil_root, "patch") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(FileDiff {
        status,
        old_path,
        new_path,
        additions,
        deletions,
        patch,
    })
}

/// Encode a FileDiff to canonical CSIL CBOR bytes.
pub fn encode_file_diff(csil_v: &FileDiff) -> Vec<u8> {
    cbor_encode(&csil_enc_file_diff(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a FileDiff.
pub fn decode_file_diff(csil_data: &[u8]) -> Result<FileDiff, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_file_diff(&csil_root)
}

/// Build the canonical CBOR value tree for a DiffResponse.
fn csil_enc_diff_response(csil_v: &DiffResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("files"),
        cbor_enc_array(&csil_v.files, csil_enc_file_diff),
    ));
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DiffResponse from a decoded CBOR value tree.
fn csil_dec_diff_response(csil_root: &CsilCborValue) -> Result<DiffResponse, CsilCborError> {
    let files = {
        let csil_field = cbor_require(csil_root, "files")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_file_diff);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(DiffResponse { files, next_cursor })
}

/// Encode a DiffResponse to canonical CSIL CBOR bytes.
pub fn encode_diff_response(csil_v: &DiffResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_diff_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DiffResponse.
pub fn decode_diff_response(csil_data: &[u8]) -> Result<DiffResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_diff_response(&csil_root)
}

/// Build the canonical CBOR value tree for a Grant.
fn csil_enc_grant(csil_v: &Grant) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("role"), cbor_text(&csil_v.role)));
    csil_entries.push((cbor_text("grantee"), cbor_text(&csil_v.grantee)));
    csil_entries.push((cbor_text("grantee_kind"), cbor_text(&csil_v.grantee_kind)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Grant from a decoded CBOR value tree.
fn csil_dec_grant(csil_root: &CsilCborValue) -> Result<Grant, CsilCborError> {
    let grantee_kind = {
        let csil_field = cbor_require(csil_root, "grantee_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let grantee = {
        let csil_field = cbor_require(csil_root, "grantee")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let role = {
        let csil_field = cbor_require(csil_root, "role")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(Grant {
        grantee_kind,
        grantee,
        role,
    })
}

/// Encode a Grant to canonical CSIL CBOR bytes.
pub fn encode_grant(csil_v: &Grant) -> Vec<u8> {
    cbor_encode(&csil_enc_grant(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Grant.
pub fn decode_grant(csil_data: &[u8]) -> Result<Grant, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_grant(&csil_root)
}

/// Build the canonical CBOR value tree for a SetGrantRequest.
fn csil_enc_set_grant_request(csil_v: &SetGrantRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("grant"), csil_enc_grant(&csil_v.grant)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SetGrantRequest from a decoded CBOR value tree.
fn csil_dec_set_grant_request(csil_root: &CsilCborValue) -> Result<SetGrantRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let grant = {
        let csil_field = cbor_require(csil_root, "grant")?;
        let csil_decode = csil_dec_grant;
        csil_decode(csil_field)?
    };
    Ok(SetGrantRequest { ctx, repo, grant })
}

/// Encode a SetGrantRequest to canonical CSIL CBOR bytes.
pub fn encode_set_grant_request(csil_v: &SetGrantRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_set_grant_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SetGrantRequest.
pub fn decode_set_grant_request(csil_data: &[u8]) -> Result<SetGrantRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_set_grant_request(&csil_root)
}

/// Build the canonical CBOR value tree for a RemoveGrantRequest.
fn csil_enc_remove_grant_request(csil_v: &RemoveGrantRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("grantee"), cbor_text(&csil_v.grantee)));
    csil_entries.push((cbor_text("grantee_kind"), cbor_text(&csil_v.grantee_kind)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RemoveGrantRequest from a decoded CBOR value tree.
fn csil_dec_remove_grant_request(
    csil_root: &CsilCborValue,
) -> Result<RemoveGrantRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let grantee_kind = {
        let csil_field = cbor_require(csil_root, "grantee_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let grantee = {
        let csil_field = cbor_require(csil_root, "grantee")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(RemoveGrantRequest {
        ctx,
        repo,
        grantee_kind,
        grantee,
    })
}

/// Encode a RemoveGrantRequest to canonical CSIL CBOR bytes.
pub fn encode_remove_grant_request(csil_v: &RemoveGrantRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_remove_grant_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RemoveGrantRequest.
pub fn decode_remove_grant_request(csil_data: &[u8]) -> Result<RemoveGrantRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_remove_grant_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListGrantsResponse.
fn csil_enc_list_grants_response(csil_v: &ListGrantsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("grants"),
        cbor_enc_array(&csil_v.grants, csil_enc_grant),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListGrantsResponse from a decoded CBOR value tree.
fn csil_dec_list_grants_response(
    csil_root: &CsilCborValue,
) -> Result<ListGrantsResponse, CsilCborError> {
    let grants = {
        let csil_field = cbor_require(csil_root, "grants")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_grant);
        csil_decode(csil_field)?
    };
    Ok(ListGrantsResponse { grants })
}

/// Encode a ListGrantsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_grants_response(csil_v: &ListGrantsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_grants_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListGrantsResponse.
pub fn decode_list_grants_response(csil_data: &[u8]) -> Result<ListGrantsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_grants_response(&csil_root)
}

/// Build the canonical CBOR value tree for a BranchProtection.
fn csil_enc_branch_protection(csil_v: &BranchProtection) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(8);
    csil_entries.push((cbor_text("ref_pattern"), cbor_text(&csil_v.ref_pattern)));
    csil_entries.push((cbor_text("allow_delete"), cbor_bool(csil_v.allow_delete)));
    csil_entries.push((cbor_text("dismiss_stale"), cbor_bool(csil_v.dismiss_stale)));
    csil_entries.push((
        cbor_text("required_checks"),
        cbor_enc_array(&csil_v.required_checks, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((
        cbor_text("allow_force_push"),
        cbor_bool(csil_v.allow_force_push),
    ));
    csil_entries.push((
        cbor_text("required_approvals"),
        cbor_uint(csil_v.required_approvals),
    ));
    csil_entries.push((
        cbor_text("allowed_merge_kinds"),
        cbor_enc_array(&csil_v.allowed_merge_kinds, |csil_elem| {
            cbor_text(csil_elem)
        }),
    ));
    csil_entries.push((
        cbor_text("required_signature_level"),
        cbor_text(&csil_v.required_signature_level),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BranchProtection from a decoded CBOR value tree.
fn csil_dec_branch_protection(
    csil_root: &CsilCborValue,
) -> Result<BranchProtection, CsilCborError> {
    let ref_pattern = {
        let csil_field = cbor_require(csil_root, "ref_pattern")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let required_approvals = {
        let csil_field = cbor_require(csil_root, "required_approvals")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let dismiss_stale = {
        let csil_field = cbor_require(csil_root, "dismiss_stale")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let required_signature_level = {
        let csil_field = cbor_require(csil_root, "required_signature_level")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let required_checks = {
        let csil_field = cbor_require(csil_root, "required_checks")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let allowed_merge_kinds = {
        let csil_field = cbor_require(csil_root, "allowed_merge_kinds")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let allow_force_push = {
        let csil_field = cbor_require(csil_root, "allow_force_push")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let allow_delete = {
        let csil_field = cbor_require(csil_root, "allow_delete")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(BranchProtection {
        ref_pattern,
        required_approvals,
        dismiss_stale,
        required_signature_level,
        required_checks,
        allowed_merge_kinds,
        allow_force_push,
        allow_delete,
    })
}

/// Encode a BranchProtection to canonical CSIL CBOR bytes.
pub fn encode_branch_protection(csil_v: &BranchProtection) -> Vec<u8> {
    cbor_encode(&csil_enc_branch_protection(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BranchProtection.
pub fn decode_branch_protection(csil_data: &[u8]) -> Result<BranchProtection, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_branch_protection(&csil_root)
}

/// Build the canonical CBOR value tree for a SetProtectionRequest.
fn csil_enc_set_protection_request(csil_v: &SetProtectionRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((
        cbor_text("protection"),
        csil_enc_branch_protection(&csil_v.protection),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SetProtectionRequest from a decoded CBOR value tree.
fn csil_dec_set_protection_request(
    csil_root: &CsilCborValue,
) -> Result<SetProtectionRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let protection = {
        let csil_field = cbor_require(csil_root, "protection")?;
        let csil_decode = csil_dec_branch_protection;
        csil_decode(csil_field)?
    };
    Ok(SetProtectionRequest {
        ctx,
        repo,
        protection,
    })
}

/// Encode a SetProtectionRequest to canonical CSIL CBOR bytes.
pub fn encode_set_protection_request(csil_v: &SetProtectionRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_set_protection_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SetProtectionRequest.
pub fn decode_set_protection_request(
    csil_data: &[u8],
) -> Result<SetProtectionRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_set_protection_request(&csil_root)
}

/// Build the canonical CBOR value tree for a RemoveProtectionRequest.
fn csil_enc_remove_protection_request(csil_v: &RemoveProtectionRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("ref_pattern"), cbor_text(&csil_v.ref_pattern)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RemoveProtectionRequest from a decoded CBOR value tree.
fn csil_dec_remove_protection_request(
    csil_root: &CsilCborValue,
) -> Result<RemoveProtectionRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let ref_pattern = {
        let csil_field = cbor_require(csil_root, "ref_pattern")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(RemoveProtectionRequest {
        ctx,
        repo,
        ref_pattern,
    })
}

/// Encode a RemoveProtectionRequest to canonical CSIL CBOR bytes.
pub fn encode_remove_protection_request(csil_v: &RemoveProtectionRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_remove_protection_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RemoveProtectionRequest.
pub fn decode_remove_protection_request(
    csil_data: &[u8],
) -> Result<RemoveProtectionRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_remove_protection_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListProtectionsResponse.
fn csil_enc_list_protections_response(csil_v: &ListProtectionsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("protections"),
        cbor_enc_array(&csil_v.protections, csil_enc_branch_protection),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListProtectionsResponse from a decoded CBOR value tree.
fn csil_dec_list_protections_response(
    csil_root: &CsilCborValue,
) -> Result<ListProtectionsResponse, CsilCborError> {
    let protections = {
        let csil_field = cbor_require(csil_root, "protections")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_branch_protection);
        csil_decode(csil_field)?
    };
    Ok(ListProtectionsResponse { protections })
}

/// Encode a ListProtectionsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_protections_response(csil_v: &ListProtectionsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_protections_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListProtectionsResponse.
pub fn decode_list_protections_response(
    csil_data: &[u8],
) -> Result<ListProtectionsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_protections_response(&csil_root)
}

/// Build the canonical CBOR value tree for a PrHead.
fn csil_enc_pr_head(csil_v: &PrHead) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ref"), cbor_text(&csil_v.r#ref)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a PrHead from a decoded CBOR value tree.
fn csil_dec_pr_head(csil_root: &CsilCborValue) -> Result<PrHead, CsilCborError> {
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let r#ref = {
        let csil_field = cbor_require(csil_root, "ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(PrHead { repo, r#ref })
}

/// Encode a PrHead to canonical CSIL CBOR bytes.
pub fn encode_pr_head(csil_v: &PrHead) -> Vec<u8> {
    cbor_encode(&csil_enc_pr_head(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a PrHead.
pub fn decode_pr_head(csil_data: &[u8]) -> Result<PrHead, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_pr_head(&csil_root)
}

/// Build the canonical CBOR value tree for a PullRequest.
fn csil_enc_pull_request(csil_v: &PullRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(18);
    if let Some(csil_inner) = &csil_v.body {
        csil_entries.push((cbor_text("body"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("head"), csil_enc_pr_head(&csil_v.head)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("title"), cbor_text(&csil_v.title)));
    csil_entries.push((cbor_text("author"), csil_enc_identity(&csil_v.author)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    csil_entries.push((cbor_text("base_ref"), cbor_text(&csil_v.base_ref)));
    csil_entries.push((cbor_text("base_sha"), cbor_text(&csil_v.base_sha)));
    csil_entries.push((cbor_text("head_sha"), cbor_text(&csil_v.head_sha)));
    if let Some(csil_inner) = &csil_v.closed_at {
        csil_entries.push((cbor_text("closed_at"), cbor_int(*csil_inner)));
    }
    csil_entries.push((cbor_text("mergeable"), cbor_text(&csil_v.mergeable)));
    if let Some(csil_inner) = &csil_v.merged_by {
        csil_entries.push((cbor_text("merged_by"), csil_enc_identity(csil_inner)));
    }
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    if let Some(csil_inner) = &csil_v.merge_kind {
        csil_entries.push((cbor_text("merge_kind"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("updated_at"), cbor_int(csil_v.updated_at)));
    if let Some(csil_inner) = &csil_v.merged_commit {
        csil_entries.push((cbor_text("merged_commit"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.merge_base_sha {
        csil_entries.push((cbor_text("merge_base_sha"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a PullRequest from a decoded CBOR value tree.
fn csil_dec_pull_request(csil_root: &CsilCborValue) -> Result<PullRequest, CsilCborError> {
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let title = {
        let csil_field = cbor_require(csil_root, "title")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let body = match cbor_map_get(csil_root, "body") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let author = {
        let csil_field = cbor_require(csil_root, "author")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let base_ref = {
        let csil_field = cbor_require(csil_root, "base_ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let head = {
        let csil_field = cbor_require(csil_root, "head")?;
        let csil_decode = csil_dec_pr_head;
        csil_decode(csil_field)?
    };
    let base_sha = {
        let csil_field = cbor_require(csil_root, "base_sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let head_sha = {
        let csil_field = cbor_require(csil_root, "head_sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let merge_base_sha = match cbor_map_get(csil_root, "merge_base_sha") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let mergeable = {
        let csil_field = cbor_require(csil_root, "mergeable")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let merge_kind = match cbor_map_get(csil_root, "merge_kind") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let merged_commit = match cbor_map_get(csil_root, "merged_commit") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let merged_by = match cbor_map_get(csil_root, "merged_by") {
        Some(csil_field) => {
            let csil_decode = csil_dec_identity;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let updated_at = {
        let csil_field = cbor_require(csil_root, "updated_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let closed_at = match cbor_map_get(csil_root, "closed_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(PullRequest {
        repo,
        number,
        title,
        body,
        author,
        state,
        base_ref,
        head,
        base_sha,
        head_sha,
        merge_base_sha,
        mergeable,
        merge_kind,
        merged_commit,
        merged_by,
        created_at,
        updated_at,
        closed_at,
    })
}

/// Encode a PullRequest to canonical CSIL CBOR bytes.
pub fn encode_pull_request(csil_v: &PullRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_pull_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a PullRequest.
pub fn decode_pull_request(csil_data: &[u8]) -> Result<PullRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_pull_request(&csil_root)
}

/// Build the canonical CBOR value tree for a PrRef.
fn csil_enc_pr_ref(csil_v: &PrRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a PrRef from a decoded CBOR value tree.
fn csil_dec_pr_ref(csil_root: &CsilCborValue) -> Result<PrRef, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    Ok(PrRef { ctx, repo, number })
}

/// Encode a PrRef to canonical CSIL CBOR bytes.
pub fn encode_pr_ref(csil_v: &PrRef) -> Vec<u8> {
    cbor_encode(&csil_enc_pr_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a PrRef.
pub fn decode_pr_ref(csil_data: &[u8]) -> Result<PrRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_pr_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a OpenPrRequest.
fn csil_enc_open_pr_request(csil_v: &OpenPrRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.body {
        csil_entries.push((cbor_text("body"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("head"), csil_enc_pr_head(&csil_v.head)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("title"), cbor_text(&csil_v.title)));
    csil_entries.push((cbor_text("base_ref"), cbor_text(&csil_v.base_ref)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a OpenPrRequest from a decoded CBOR value tree.
fn csil_dec_open_pr_request(csil_root: &CsilCborValue) -> Result<OpenPrRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let title = {
        let csil_field = cbor_require(csil_root, "title")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let body = match cbor_map_get(csil_root, "body") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let base_ref = {
        let csil_field = cbor_require(csil_root, "base_ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let head = {
        let csil_field = cbor_require(csil_root, "head")?;
        let csil_decode = csil_dec_pr_head;
        csil_decode(csil_field)?
    };
    Ok(OpenPrRequest {
        ctx,
        repo,
        title,
        body,
        base_ref,
        head,
    })
}

/// Encode a OpenPrRequest to canonical CSIL CBOR bytes.
pub fn encode_open_pr_request(csil_v: &OpenPrRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_open_pr_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a OpenPrRequest.
pub fn decode_open_pr_request(csil_data: &[u8]) -> Result<OpenPrRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_open_pr_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListPrRequest.
fn csil_enc_list_pr_request(csil_v: &ListPrRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.state {
        csil_entries.push((cbor_text("state"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListPrRequest from a decoded CBOR value tree.
fn csil_dec_list_pr_request(csil_root: &CsilCborValue) -> Result<ListPrRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let state = match cbor_map_get(csil_root, "state") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ListPrRequest {
        ctx,
        repo,
        state,
        page,
    })
}

/// Encode a ListPrRequest to canonical CSIL CBOR bytes.
pub fn encode_list_pr_request(csil_v: &ListPrRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_pr_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListPrRequest.
pub fn decode_list_pr_request(csil_data: &[u8]) -> Result<ListPrRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_pr_request(&csil_root)
}

/// Build the canonical CBOR value tree for a PrPage.
fn csil_enc_pr_page(csil_v: &PrPage) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    csil_entries.push((
        cbor_text("pull_requests"),
        cbor_enc_array(&csil_v.pull_requests, csil_enc_pull_request),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a PrPage from a decoded CBOR value tree.
fn csil_dec_pr_page(csil_root: &CsilCborValue) -> Result<PrPage, CsilCborError> {
    let pull_requests = {
        let csil_field = cbor_require(csil_root, "pull_requests")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_pull_request);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(PrPage {
        pull_requests,
        next_cursor,
    })
}

/// Encode a PrPage to canonical CSIL CBOR bytes.
pub fn encode_pr_page(csil_v: &PrPage) -> Vec<u8> {
    cbor_encode(&csil_enc_pr_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a PrPage.
pub fn decode_pr_page(csil_data: &[u8]) -> Result<PrPage, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_pr_page(&csil_root)
}

/// Build the canonical CBOR value tree for a Review.
fn csil_enc_review(csil_v: &Review) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    if let Some(csil_inner) = &csil_v.body {
        csil_entries.push((cbor_text("body"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    csil_entries.push((cbor_text("head_sha"), cbor_text(&csil_v.head_sha)));
    csil_entries.push((cbor_text("reviewer"), csil_enc_identity(&csil_v.reviewer)));
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Review from a decoded CBOR value tree.
fn csil_dec_review(csil_root: &CsilCborValue) -> Result<Review, CsilCborError> {
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let reviewer = {
        let csil_field = cbor_require(csil_root, "reviewer")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let head_sha = {
        let csil_field = cbor_require(csil_root, "head_sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let body = match cbor_map_get(csil_root, "body") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(Review {
        number,
        reviewer,
        state,
        head_sha,
        body,
        created_at,
    })
}

/// Encode a Review to canonical CSIL CBOR bytes.
pub fn encode_review(csil_v: &Review) -> Vec<u8> {
    cbor_encode(&csil_enc_review(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Review.
pub fn decode_review(csil_data: &[u8]) -> Result<Review, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_review(&csil_root)
}

/// Build the canonical CBOR value tree for a ReviewRequest.
fn csil_enc_review_request(csil_v: &ReviewRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.body {
        csil_entries.push((cbor_text("body"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    csil_entries.push((cbor_text("head_sha"), cbor_text(&csil_v.head_sha)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ReviewRequest from a decoded CBOR value tree.
fn csil_dec_review_request(csil_root: &CsilCborValue) -> Result<ReviewRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let body = match cbor_map_get(csil_root, "body") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let head_sha = {
        let csil_field = cbor_require(csil_root, "head_sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(ReviewRequest {
        ctx,
        repo,
        number,
        state,
        body,
        head_sha,
    })
}

/// Encode a ReviewRequest to canonical CSIL CBOR bytes.
pub fn encode_review_request(csil_v: &ReviewRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_review_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ReviewRequest.
pub fn decode_review_request(csil_data: &[u8]) -> Result<ReviewRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_review_request(&csil_root)
}

/// Build the canonical CBOR value tree for a Comment.
fn csil_enc_comment(csil_v: &Comment) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(9);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("body"), cbor_text(&csil_v.body)));
    if let Some(csil_inner) = &csil_v.line {
        csil_entries.push((cbor_text("line"), cbor_uint(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.path {
        csil_entries.push((cbor_text("path"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("author"), csil_enc_identity(&csil_v.author)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    if let Some(csil_inner) = &csil_v.edited_at {
        csil_entries.push((cbor_text("edited_at"), cbor_int(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.commit_sha {
        csil_entries.push((cbor_text("commit_sha"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Comment from a decoded CBOR value tree.
fn csil_dec_comment(csil_root: &CsilCborValue) -> Result<Comment, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let author = {
        let csil_field = cbor_require(csil_root, "author")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let body = {
        let csil_field = cbor_require(csil_root, "body")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let path = match cbor_map_get(csil_root, "path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let line = match cbor_map_get(csil_root, "line") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let commit_sha = match cbor_map_get(csil_root, "commit_sha") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let edited_at = match cbor_map_get(csil_root, "edited_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Comment {
        id,
        number,
        author,
        body,
        path,
        line,
        commit_sha,
        created_at,
        edited_at,
    })
}

/// Encode a Comment to canonical CSIL CBOR bytes.
pub fn encode_comment(csil_v: &Comment) -> Vec<u8> {
    cbor_encode(&csil_enc_comment(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Comment.
pub fn decode_comment(csil_data: &[u8]) -> Result<Comment, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_comment(&csil_root)
}

/// Build the canonical CBOR value tree for a CommentRequest.
fn csil_enc_comment_request(csil_v: &CommentRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("body"), cbor_text(&csil_v.body)));
    if let Some(csil_inner) = &csil_v.line {
        csil_entries.push((cbor_text("line"), cbor_uint(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.path {
        csil_entries.push((cbor_text("path"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    if let Some(csil_inner) = &csil_v.commit_sha {
        csil_entries.push((cbor_text("commit_sha"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CommentRequest from a decoded CBOR value tree.
fn csil_dec_comment_request(csil_root: &CsilCborValue) -> Result<CommentRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let body = {
        let csil_field = cbor_require(csil_root, "body")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let path = match cbor_map_get(csil_root, "path") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let line = match cbor_map_get(csil_root, "line") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let commit_sha = match cbor_map_get(csil_root, "commit_sha") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CommentRequest {
        ctx,
        repo,
        number,
        body,
        path,
        line,
        commit_sha,
    })
}

/// Encode a CommentRequest to canonical CSIL CBOR bytes.
pub fn encode_comment_request(csil_v: &CommentRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_comment_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CommentRequest.
pub fn decode_comment_request(csil_data: &[u8]) -> Result<CommentRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_comment_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListCommentsResponse.
fn csil_enc_list_comments_response(csil_v: &ListCommentsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("comments"),
        cbor_enc_array(&csil_v.comments, csil_enc_comment),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListCommentsResponse from a decoded CBOR value tree.
fn csil_dec_list_comments_response(
    csil_root: &CsilCborValue,
) -> Result<ListCommentsResponse, CsilCborError> {
    let comments = {
        let csil_field = cbor_require(csil_root, "comments")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_comment);
        csil_decode(csil_field)?
    };
    Ok(ListCommentsResponse { comments })
}

/// Encode a ListCommentsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_comments_response(csil_v: &ListCommentsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_comments_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListCommentsResponse.
pub fn decode_list_comments_response(
    csil_data: &[u8],
) -> Result<ListCommentsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_comments_response(&csil_root)
}

/// Build the canonical CBOR value tree for a ListReviewsResponse.
fn csil_enc_list_reviews_response(csil_v: &ListReviewsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("reviews"),
        cbor_enc_array(&csil_v.reviews, csil_enc_review),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListReviewsResponse from a decoded CBOR value tree.
fn csil_dec_list_reviews_response(
    csil_root: &CsilCborValue,
) -> Result<ListReviewsResponse, CsilCborError> {
    let reviews = {
        let csil_field = cbor_require(csil_root, "reviews")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_review);
        csil_decode(csil_field)?
    };
    Ok(ListReviewsResponse { reviews })
}

/// Encode a ListReviewsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_reviews_response(csil_v: &ListReviewsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_reviews_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListReviewsResponse.
pub fn decode_list_reviews_response(
    csil_data: &[u8],
) -> Result<ListReviewsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_reviews_response(&csil_root)
}

/// Build the canonical CBOR value tree for a MergeRequest.
fn csil_enc_merge_request(csil_v: &MergeRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    if let Some(csil_inner) = &csil_v.message {
        csil_entries.push((cbor_text("message"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("merge_kind"), cbor_text(&csil_v.merge_kind)));
    csil_entries.push((
        cbor_text("expected_head_sha"),
        cbor_text(&csil_v.expected_head_sha),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a MergeRequest from a decoded CBOR value tree.
fn csil_dec_merge_request(csil_root: &CsilCborValue) -> Result<MergeRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let expected_head_sha = {
        let csil_field = cbor_require(csil_root, "expected_head_sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let merge_kind = {
        let csil_field = cbor_require(csil_root, "merge_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let message = match cbor_map_get(csil_root, "message") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(MergeRequest {
        ctx,
        repo,
        number,
        expected_head_sha,
        merge_kind,
        message,
    })
}

/// Encode a MergeRequest to canonical CSIL CBOR bytes.
pub fn encode_merge_request(csil_v: &MergeRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_merge_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a MergeRequest.
pub fn decode_merge_request(csil_data: &[u8]) -> Result<MergeRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_merge_request(&csil_root)
}

/// Build the canonical CBOR value tree for a MergeResult.
fn csil_enc_merge_result(csil_v: &MergeResult) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("pull_request"),
        csil_enc_pull_request(&csil_v.pull_request),
    ));
    csil_entries.push((cbor_text("merged_commit"), cbor_text(&csil_v.merged_commit)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a MergeResult from a decoded CBOR value tree.
fn csil_dec_merge_result(csil_root: &CsilCborValue) -> Result<MergeResult, CsilCborError> {
    let pull_request = {
        let csil_field = cbor_require(csil_root, "pull_request")?;
        let csil_decode = csil_dec_pull_request;
        csil_decode(csil_field)?
    };
    let merged_commit = {
        let csil_field = cbor_require(csil_root, "merged_commit")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(MergeResult {
        pull_request,
        merged_commit,
    })
}

/// Encode a MergeResult to canonical CSIL CBOR bytes.
pub fn encode_merge_result(csil_v: &MergeResult) -> Vec<u8> {
    cbor_encode(&csil_enc_merge_result(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a MergeResult.
pub fn decode_merge_result(csil_data: &[u8]) -> Result<MergeResult, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_merge_result(&csil_root)
}

/// Build the canonical CBOR value tree for a RetargetRequest.
fn csil_enc_retarget_request(csil_v: &RetargetRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    csil_entries.push((cbor_text("base_ref"), cbor_text(&csil_v.base_ref)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RetargetRequest from a decoded CBOR value tree.
fn csil_dec_retarget_request(csil_root: &CsilCborValue) -> Result<RetargetRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let base_ref = {
        let csil_field = cbor_require(csil_root, "base_ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(RetargetRequest {
        ctx,
        repo,
        number,
        base_ref,
    })
}

/// Encode a RetargetRequest to canonical CSIL CBOR bytes.
pub fn encode_retarget_request(csil_v: &RetargetRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_retarget_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RetargetRequest.
pub fn decode_retarget_request(csil_data: &[u8]) -> Result<RetargetRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_retarget_request(&csil_root)
}

/// Build the canonical CBOR value tree for a UpdatePrRequest.
fn csil_enc_update_pr_request(csil_v: &UpdatePrRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.body {
        csil_entries.push((cbor_text("body"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.title {
        csil_entries.push((cbor_text("title"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UpdatePrRequest from a decoded CBOR value tree.
fn csil_dec_update_pr_request(csil_root: &CsilCborValue) -> Result<UpdatePrRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let title = match cbor_map_get(csil_root, "title") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let body = match cbor_map_get(csil_root, "body") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(UpdatePrRequest {
        ctx,
        repo,
        number,
        title,
        body,
    })
}

/// Encode a UpdatePrRequest to canonical CSIL CBOR bytes.
pub fn encode_update_pr_request(csil_v: &UpdatePrRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_update_pr_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UpdatePrRequest.
pub fn decode_update_pr_request(csil_data: &[u8]) -> Result<UpdatePrRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_update_pr_request(&csil_root)
}

/// Build the canonical CBOR value tree for a CommitStatus.
fn csil_enc_commit_status(csil_v: &CommitStatus) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("sha"), cbor_text(&csil_v.sha)));
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("context"), cbor_text(&csil_v.context)));
    csil_entries.push((cbor_text("reporter"), csil_enc_identity(&csil_v.reporter)));
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    if let Some(csil_inner) = &csil_v.target_url {
        csil_entries.push((cbor_text("target_url"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.description {
        csil_entries.push((cbor_text("description"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CommitStatus from a decoded CBOR value tree.
fn csil_dec_commit_status(csil_root: &CsilCborValue) -> Result<CommitStatus, CsilCborError> {
    let sha = {
        let csil_field = cbor_require(csil_root, "sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let context = {
        let csil_field = cbor_require(csil_root, "context")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let target_url = match cbor_map_get(csil_root, "target_url") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let description = match cbor_map_get(csil_root, "description") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let reporter = {
        let csil_field = cbor_require(csil_root, "reporter")?;
        let csil_decode = csil_dec_identity;
        csil_decode(csil_field)?
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(CommitStatus {
        sha,
        context,
        state,
        target_url,
        description,
        reporter,
        created_at,
    })
}

/// Encode a CommitStatus to canonical CSIL CBOR bytes.
pub fn encode_commit_status(csil_v: &CommitStatus) -> Vec<u8> {
    cbor_encode(&csil_enc_commit_status(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CommitStatus.
pub fn decode_commit_status(csil_data: &[u8]) -> Result<CommitStatus, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_commit_status(&csil_root)
}

/// Build the canonical CBOR value tree for a SetStatusRequest.
fn csil_enc_set_status_request(csil_v: &SetStatusRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("sha"), cbor_text(&csil_v.sha)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("context"), cbor_text(&csil_v.context)));
    if let Some(csil_inner) = &csil_v.target_url {
        csil_entries.push((cbor_text("target_url"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.description {
        csil_entries.push((cbor_text("description"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SetStatusRequest from a decoded CBOR value tree.
fn csil_dec_set_status_request(
    csil_root: &CsilCborValue,
) -> Result<SetStatusRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let sha = {
        let csil_field = cbor_require(csil_root, "sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let context = {
        let csil_field = cbor_require(csil_root, "context")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let target_url = match cbor_map_get(csil_root, "target_url") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let description = match cbor_map_get(csil_root, "description") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(SetStatusRequest {
        ctx,
        repo,
        sha,
        context,
        state,
        target_url,
        description,
    })
}

/// Encode a SetStatusRequest to canonical CSIL CBOR bytes.
pub fn encode_set_status_request(csil_v: &SetStatusRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_set_status_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SetStatusRequest.
pub fn decode_set_status_request(csil_data: &[u8]) -> Result<SetStatusRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_set_status_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListStatusRequest.
fn csil_enc_list_status_request(csil_v: &ListStatusRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("sha"), cbor_text(&csil_v.sha)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListStatusRequest from a decoded CBOR value tree.
fn csil_dec_list_status_request(
    csil_root: &CsilCborValue,
) -> Result<ListStatusRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let sha = {
        let csil_field = cbor_require(csil_root, "sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(ListStatusRequest { ctx, repo, sha })
}

/// Encode a ListStatusRequest to canonical CSIL CBOR bytes.
pub fn encode_list_status_request(csil_v: &ListStatusRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_status_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListStatusRequest.
pub fn decode_list_status_request(csil_data: &[u8]) -> Result<ListStatusRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_status_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListStatusResponse.
fn csil_enc_list_status_response(csil_v: &ListStatusResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("statuses"),
        cbor_enc_array(&csil_v.statuses, csil_enc_commit_status),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListStatusResponse from a decoded CBOR value tree.
fn csil_dec_list_status_response(
    csil_root: &CsilCborValue,
) -> Result<ListStatusResponse, CsilCborError> {
    let statuses = {
        let csil_field = cbor_require(csil_root, "statuses")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_commit_status);
        csil_decode(csil_field)?
    };
    Ok(ListStatusResponse { statuses })
}

/// Encode a ListStatusResponse to canonical CSIL CBOR bytes.
pub fn encode_list_status_response(csil_v: &ListStatusResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_status_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListStatusResponse.
pub fn decode_list_status_response(csil_data: &[u8]) -> Result<ListStatusResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_status_response(&csil_root)
}

/// Build the canonical CBOR value tree for a Endpoint.
fn csil_enc_endpoint(csil_v: &Endpoint) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(11);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.url {
        csil_entries.push((cbor_text("url"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("kind"), cbor_text(&csil_v.kind)));
    if let Some(csil_inner) = &csil_v.repo {
        csil_entries.push((cbor_text("repo"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("active"), cbor_bool(csil_v.active)));
    csil_entries.push((cbor_text("unordered"), cbor_bool(csil_v.unordered)));
    csil_entries.push((cbor_text("created_at"), cbor_int(csil_v.created_at)));
    csil_entries.push((cbor_text("content_type"), cbor_text(&csil_v.content_type)));
    csil_entries.push((
        cbor_text("event_filter"),
        cbor_enc_array(&csil_v.event_filter, |csil_elem| cbor_text(csil_elem)),
    ));
    if let Some(csil_inner) = &csil_v.corndogs_queue {
        csil_entries.push((cbor_text("corndogs_queue"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Endpoint from a decoded CBOR value tree.
fn csil_dec_endpoint(csil_root: &CsilCborValue) -> Result<Endpoint, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let repo = match cbor_map_get(csil_root, "repo") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let kind = {
        let csil_field = cbor_require(csil_root, "kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let url = match cbor_map_get(csil_root, "url") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let corndogs_queue = match cbor_map_get(csil_root, "corndogs_queue") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let event_filter = {
        let csil_field = cbor_require(csil_root, "event_filter")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let content_type = {
        let csil_field = cbor_require(csil_root, "content_type")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let active = {
        let csil_field = cbor_require(csil_root, "active")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let unordered = {
        let csil_field = cbor_require(csil_root, "unordered")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let created_at = {
        let csil_field = cbor_require(csil_root, "created_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(Endpoint {
        id,
        org,
        repo,
        kind,
        url,
        corndogs_queue,
        event_filter,
        content_type,
        active,
        unordered,
        created_at,
    })
}

/// Encode a Endpoint to canonical CSIL CBOR bytes.
pub fn encode_endpoint(csil_v: &Endpoint) -> Vec<u8> {
    cbor_encode(&csil_enc_endpoint(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Endpoint.
pub fn decode_endpoint(csil_data: &[u8]) -> Result<Endpoint, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_endpoint(&csil_root)
}

/// Build the canonical CBOR value tree for a CreateEndpointRequest.
fn csil_enc_create_endpoint_request(csil_v: &CreateEndpointRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(10);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.url {
        csil_entries.push((cbor_text("url"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("kind"), cbor_text(&csil_v.kind)));
    if let Some(csil_inner) = &csil_v.repo {
        csil_entries.push((cbor_text("repo"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.secret {
        csil_entries.push((cbor_text("secret"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.unordered {
        csil_entries.push((cbor_text("unordered"), cbor_bool(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.content_type {
        csil_entries.push((cbor_text("content_type"), cbor_text(csil_inner)));
    }
    csil_entries.push((
        cbor_text("event_filter"),
        cbor_enc_array(&csil_v.event_filter, |csil_elem| cbor_text(csil_elem)),
    ));
    if let Some(csil_inner) = &csil_v.corndogs_queue {
        csil_entries.push((cbor_text("corndogs_queue"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CreateEndpointRequest from a decoded CBOR value tree.
fn csil_dec_create_endpoint_request(
    csil_root: &CsilCborValue,
) -> Result<CreateEndpointRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let repo = match cbor_map_get(csil_root, "repo") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let kind = {
        let csil_field = cbor_require(csil_root, "kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let url = match cbor_map_get(csil_root, "url") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let corndogs_queue = match cbor_map_get(csil_root, "corndogs_queue") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let event_filter = {
        let csil_field = cbor_require(csil_root, "event_filter")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let content_type = match cbor_map_get(csil_root, "content_type") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let secret = match cbor_map_get(csil_root, "secret") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let unordered = match cbor_map_get(csil_root, "unordered") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bool;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(CreateEndpointRequest {
        ctx,
        org,
        repo,
        kind,
        url,
        corndogs_queue,
        event_filter,
        content_type,
        secret,
        unordered,
    })
}

/// Encode a CreateEndpointRequest to canonical CSIL CBOR bytes.
pub fn encode_create_endpoint_request(csil_v: &CreateEndpointRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_create_endpoint_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CreateEndpointRequest.
pub fn decode_create_endpoint_request(
    csil_data: &[u8],
) -> Result<CreateEndpointRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_create_endpoint_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListEndpointsRequest.
fn csil_enc_list_endpoints_request(csil_v: &ListEndpointsRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("target"), csil_enc_org_repo_ref(&csil_v.target)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListEndpointsRequest from a decoded CBOR value tree.
fn csil_dec_list_endpoints_request(
    csil_root: &CsilCborValue,
) -> Result<ListEndpointsRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let target = {
        let csil_field = cbor_require(csil_root, "target")?;
        let csil_decode = csil_dec_org_repo_ref;
        csil_decode(csil_field)?
    };
    Ok(ListEndpointsRequest { ctx, target })
}

/// Encode a ListEndpointsRequest to canonical CSIL CBOR bytes.
pub fn encode_list_endpoints_request(csil_v: &ListEndpointsRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_endpoints_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListEndpointsRequest.
pub fn decode_list_endpoints_request(
    csil_data: &[u8],
) -> Result<ListEndpointsRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_endpoints_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListEndpointsResponse.
fn csil_enc_list_endpoints_response(csil_v: &ListEndpointsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("endpoints"),
        cbor_enc_array(&csil_v.endpoints, csil_enc_endpoint),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListEndpointsResponse from a decoded CBOR value tree.
fn csil_dec_list_endpoints_response(
    csil_root: &CsilCborValue,
) -> Result<ListEndpointsResponse, CsilCborError> {
    let endpoints = {
        let csil_field = cbor_require(csil_root, "endpoints")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_endpoint);
        csil_decode(csil_field)?
    };
    Ok(ListEndpointsResponse { endpoints })
}

/// Encode a ListEndpointsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_endpoints_response(csil_v: &ListEndpointsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_endpoints_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListEndpointsResponse.
pub fn decode_list_endpoints_response(
    csil_data: &[u8],
) -> Result<ListEndpointsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_endpoints_response(&csil_root)
}

/// Build the canonical CBOR value tree for a EndpointRef.
fn csil_enc_endpoint_ref(csil_v: &EndpointRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("endpoint_id"), cbor_text(&csil_v.endpoint_id)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a EndpointRef from a decoded CBOR value tree.
fn csil_dec_endpoint_ref(csil_root: &CsilCborValue) -> Result<EndpointRef, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let endpoint_id = {
        let csil_field = cbor_require(csil_root, "endpoint_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(EndpointRef { ctx, endpoint_id })
}

/// Encode a EndpointRef to canonical CSIL CBOR bytes.
pub fn encode_endpoint_ref(csil_v: &EndpointRef) -> Vec<u8> {
    cbor_encode(&csil_enc_endpoint_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a EndpointRef.
pub fn decode_endpoint_ref(csil_data: &[u8]) -> Result<EndpointRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_endpoint_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a UpdateEndpointRequest.
fn csil_enc_update_endpoint_request(csil_v: &UpdateEndpointRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.url {
        csil_entries.push((cbor_text("url"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.active {
        csil_entries.push((cbor_text("active"), cbor_bool(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.secret {
        csil_entries.push((cbor_text("secret"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.unordered {
        csil_entries.push((cbor_text("unordered"), cbor_bool(*csil_inner)));
    }
    csil_entries.push((cbor_text("endpoint_id"), cbor_text(&csil_v.endpoint_id)));
    if let Some(csil_inner) = &csil_v.event_filter {
        csil_entries.push((
            cbor_text("event_filter"),
            cbor_enc_array(csil_inner, |csil_elem| cbor_text(csil_elem)),
        ));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UpdateEndpointRequest from a decoded CBOR value tree.
fn csil_dec_update_endpoint_request(
    csil_root: &CsilCborValue,
) -> Result<UpdateEndpointRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let endpoint_id = {
        let csil_field = cbor_require(csil_root, "endpoint_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let url = match cbor_map_get(csil_root, "url") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let event_filter = match cbor_map_get(csil_root, "event_filter") {
        Some(csil_field) => {
            let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let active = match cbor_map_get(csil_root, "active") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bool;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let secret = match cbor_map_get(csil_root, "secret") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let unordered = match cbor_map_get(csil_root, "unordered") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bool;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(UpdateEndpointRequest {
        ctx,
        endpoint_id,
        url,
        event_filter,
        active,
        secret,
        unordered,
    })
}

/// Encode a UpdateEndpointRequest to canonical CSIL CBOR bytes.
pub fn encode_update_endpoint_request(csil_v: &UpdateEndpointRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_update_endpoint_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UpdateEndpointRequest.
pub fn decode_update_endpoint_request(
    csil_data: &[u8],
) -> Result<UpdateEndpointRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_update_endpoint_request(&csil_root)
}

/// Build the canonical CBOR value tree for a Delivery.
fn csil_enc_delivery(csil_v: &Delivery) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(11);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("attempts"), cbor_uint(csil_v.attempts)));
    csil_entries.push((cbor_text("event_seq"), cbor_uint(csil_v.event_seq)));
    csil_entries.push((cbor_text("event_kind"), cbor_text(&csil_v.event_kind)));
    csil_entries.push((cbor_text("event_lane"), cbor_text(&csil_v.event_lane)));
    if let Some(csil_inner) = &csil_v.last_error {
        csil_entries.push((cbor_text("last_error"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("endpoint_id"), cbor_text(&csil_v.endpoint_id)));
    if let Some(csil_inner) = &csil_v.last_status {
        csil_entries.push((cbor_text("last_status"), cbor_uint(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.delivered_at {
        csil_entries.push((cbor_text("delivered_at"), cbor_int(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.next_attempt_at {
        csil_entries.push((cbor_text("next_attempt_at"), cbor_int(*csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Delivery from a decoded CBOR value tree.
fn csil_dec_delivery(csil_root: &CsilCborValue) -> Result<Delivery, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let endpoint_id = {
        let csil_field = cbor_require(csil_root, "endpoint_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let event_lane = {
        let csil_field = cbor_require(csil_root, "event_lane")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let event_seq = {
        let csil_field = cbor_require(csil_root, "event_seq")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let event_kind = {
        let csil_field = cbor_require(csil_root, "event_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let attempts = {
        let csil_field = cbor_require(csil_root, "attempts")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let next_attempt_at = match cbor_map_get(csil_root, "next_attempt_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let last_status = match cbor_map_get(csil_root, "last_status") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let last_error = match cbor_map_get(csil_root, "last_error") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let delivered_at = match cbor_map_get(csil_root, "delivered_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(Delivery {
        id,
        endpoint_id,
        event_lane,
        event_seq,
        event_kind,
        state,
        attempts,
        next_attempt_at,
        last_status,
        last_error,
        delivered_at,
    })
}

/// Encode a Delivery to canonical CSIL CBOR bytes.
pub fn encode_delivery(csil_v: &Delivery) -> Vec<u8> {
    cbor_encode(&csil_enc_delivery(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Delivery.
pub fn decode_delivery(csil_data: &[u8]) -> Result<Delivery, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_delivery(&csil_root)
}

/// Build the canonical CBOR value tree for a DeliveryQuery.
fn csil_enc_delivery_query(csil_v: &DeliveryQuery) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.page {
        csil_entries.push((cbor_text("page"), csil_enc_page(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.state {
        csil_entries.push((cbor_text("state"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("endpoint_id"), cbor_text(&csil_v.endpoint_id)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DeliveryQuery from a decoded CBOR value tree.
fn csil_dec_delivery_query(csil_root: &CsilCborValue) -> Result<DeliveryQuery, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let endpoint_id = {
        let csil_field = cbor_require(csil_root, "endpoint_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let state = match cbor_map_get(csil_root, "state") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let page = match cbor_map_get(csil_root, "page") {
        Some(csil_field) => {
            let csil_decode = csil_dec_page;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(DeliveryQuery {
        ctx,
        endpoint_id,
        state,
        page,
    })
}

/// Encode a DeliveryQuery to canonical CSIL CBOR bytes.
pub fn encode_delivery_query(csil_v: &DeliveryQuery) -> Vec<u8> {
    cbor_encode(&csil_enc_delivery_query(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DeliveryQuery.
pub fn decode_delivery_query(csil_data: &[u8]) -> Result<DeliveryQuery, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_delivery_query(&csil_root)
}

/// Build the canonical CBOR value tree for a DeliveryPage.
fn csil_enc_delivery_page(csil_v: &DeliveryPage) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((
        cbor_text("deliveries"),
        cbor_enc_array(&csil_v.deliveries, csil_enc_delivery),
    ));
    if let Some(csil_inner) = &csil_v.next_cursor {
        csil_entries.push((cbor_text("next_cursor"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DeliveryPage from a decoded CBOR value tree.
fn csil_dec_delivery_page(csil_root: &CsilCborValue) -> Result<DeliveryPage, CsilCborError> {
    let deliveries = {
        let csil_field = cbor_require(csil_root, "deliveries")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_delivery);
        csil_decode(csil_field)?
    };
    let next_cursor = match cbor_map_get(csil_root, "next_cursor") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(DeliveryPage {
        deliveries,
        next_cursor,
    })
}

/// Encode a DeliveryPage to canonical CSIL CBOR bytes.
pub fn encode_delivery_page(csil_v: &DeliveryPage) -> Vec<u8> {
    cbor_encode(&csil_enc_delivery_page(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DeliveryPage.
pub fn decode_delivery_page(csil_data: &[u8]) -> Result<DeliveryPage, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_delivery_page(&csil_root)
}

/// Build the canonical CBOR value tree for a DeliveryRef.
fn csil_enc_delivery_ref(csil_v: &DeliveryRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("delivery_id"), cbor_text(&csil_v.delivery_id)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a DeliveryRef from a decoded CBOR value tree.
fn csil_dec_delivery_ref(csil_root: &CsilCborValue) -> Result<DeliveryRef, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let delivery_id = {
        let csil_field = cbor_require(csil_root, "delivery_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(DeliveryRef { ctx, delivery_id })
}

/// Encode a DeliveryRef to canonical CSIL CBOR bytes.
pub fn encode_delivery_ref(csil_v: &DeliveryRef) -> Vec<u8> {
    cbor_encode(&csil_enc_delivery_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a DeliveryRef.
pub fn decode_delivery_ref(csil_data: &[u8]) -> Result<DeliveryRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_delivery_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a MirrorStatus.
fn csil_enc_mirror_status(csil_v: &MirrorStatus) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(10);
    csil_entries.push((cbor_text("mode"), cbor_text(&csil_v.mode)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("paused"), cbor_bool(csil_v.paused)));
    csil_entries.push((cbor_text("blocked"), cbor_uint(csil_v.blocked)));
    csil_entries.push((cbor_text("pending"), cbor_uint(csil_v.pending)));
    csil_entries.push((cbor_text("cursor_seq"), cbor_uint(csil_v.cursor_seq)));
    if let Some(csil_inner) = &csil_v.last_error {
        csil_entries.push((cbor_text("last_error"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.github_repo {
        csil_entries.push((cbor_text("github_repo"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.last_success_at {
        csil_entries.push((cbor_text("last_success_at"), cbor_int(*csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.last_full_reconcile_at {
        csil_entries.push((cbor_text("last_full_reconcile_at"), cbor_int(*csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a MirrorStatus from a decoded CBOR value tree.
fn csil_dec_mirror_status(csil_root: &CsilCborValue) -> Result<MirrorStatus, CsilCborError> {
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let github_repo = match cbor_map_get(csil_root, "github_repo") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let mode = {
        let csil_field = cbor_require(csil_root, "mode")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let paused = {
        let csil_field = cbor_require(csil_root, "paused")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let cursor_seq = {
        let csil_field = cbor_require(csil_root, "cursor_seq")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let pending = {
        let csil_field = cbor_require(csil_root, "pending")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let blocked = {
        let csil_field = cbor_require(csil_root, "blocked")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let last_success_at = match cbor_map_get(csil_root, "last_success_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let last_error = match cbor_map_get(csil_root, "last_error") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let last_full_reconcile_at = match cbor_map_get(csil_root, "last_full_reconcile_at") {
        Some(csil_field) => {
            let csil_decode = cbor_as_i64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(MirrorStatus {
        repo,
        github_repo,
        mode,
        paused,
        cursor_seq,
        pending,
        blocked,
        last_success_at,
        last_error,
        last_full_reconcile_at,
    })
}

/// Encode a MirrorStatus to canonical CSIL CBOR bytes.
pub fn encode_mirror_status(csil_v: &MirrorStatus) -> Vec<u8> {
    cbor_encode(&csil_enc_mirror_status(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a MirrorStatus.
pub fn decode_mirror_status(csil_data: &[u8]) -> Result<MirrorStatus, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_mirror_status(&csil_root)
}

/// Build the canonical CBOR value tree for a BlockedAction.
fn csil_enc_blocked_action(csil_v: &BlockedAction) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("error"), cbor_text(&csil_v.error)));
    csil_entries.push((cbor_text("event_seq"), cbor_uint(csil_v.event_seq)));
    csil_entries.push((cbor_text("blocked_at"), cbor_int(csil_v.blocked_at)));
    csil_entries.push((cbor_text("object_key"), cbor_text(&csil_v.object_key)));
    csil_entries.push((cbor_text("action_kind"), cbor_text(&csil_v.action_kind)));
    csil_entries.push((
        cbor_text("credential_purpose"),
        cbor_text(&csil_v.credential_purpose),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a BlockedAction from a decoded CBOR value tree.
fn csil_dec_blocked_action(csil_root: &CsilCborValue) -> Result<BlockedAction, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let event_seq = {
        let csil_field = cbor_require(csil_root, "event_seq")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let action_kind = {
        let csil_field = cbor_require(csil_root, "action_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let object_key = {
        let csil_field = cbor_require(csil_root, "object_key")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let error = {
        let csil_field = cbor_require(csil_root, "error")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let blocked_at = {
        let csil_field = cbor_require(csil_root, "blocked_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let credential_purpose = {
        let csil_field = cbor_require(csil_root, "credential_purpose")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(BlockedAction {
        id,
        event_seq,
        action_kind,
        object_key,
        error,
        blocked_at,
        credential_purpose,
    })
}

/// Encode a BlockedAction to canonical CSIL CBOR bytes.
pub fn encode_blocked_action(csil_v: &BlockedAction) -> Vec<u8> {
    cbor_encode(&csil_enc_blocked_action(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a BlockedAction.
pub fn decode_blocked_action(csil_data: &[u8]) -> Result<BlockedAction, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_blocked_action(&csil_root)
}

/// Build the canonical CBOR value tree for a ListBlockedResponse.
fn csil_enc_list_blocked_response(csil_v: &ListBlockedResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("actions"),
        cbor_enc_array(&csil_v.actions, csil_enc_blocked_action),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListBlockedResponse from a decoded CBOR value tree.
fn csil_dec_list_blocked_response(
    csil_root: &CsilCborValue,
) -> Result<ListBlockedResponse, CsilCborError> {
    let actions = {
        let csil_field = cbor_require(csil_root, "actions")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_blocked_action);
        csil_decode(csil_field)?
    };
    Ok(ListBlockedResponse { actions })
}

/// Encode a ListBlockedResponse to canonical CSIL CBOR bytes.
pub fn encode_list_blocked_response(csil_v: &ListBlockedResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_blocked_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListBlockedResponse.
pub fn decode_list_blocked_response(
    csil_data: &[u8],
) -> Result<ListBlockedResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_blocked_response(&csil_root)
}

/// Build the canonical CBOR value tree for a ReplayRequest.
fn csil_enc_replay_request(csil_v: &ReplayRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((
        cbor_text("action_ids"),
        cbor_enc_array(&csil_v.action_ids, |csil_elem| cbor_text(csil_elem)),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ReplayRequest from a decoded CBOR value tree.
fn csil_dec_replay_request(csil_root: &CsilCborValue) -> Result<ReplayRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let action_ids = {
        let csil_field = cbor_require(csil_root, "action_ids")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(ReplayRequest {
        ctx,
        repo,
        action_ids,
    })
}

/// Encode a ReplayRequest to canonical CSIL CBOR bytes.
pub fn encode_replay_request(csil_v: &ReplayRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_replay_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ReplayRequest.
pub fn decode_replay_request(csil_data: &[u8]) -> Result<ReplayRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_replay_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ReplayResult.
fn csil_enc_replay_result(csil_v: &ReplayResult) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((cbor_text("requeued"), cbor_uint(csil_v.requeued)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ReplayResult from a decoded CBOR value tree.
fn csil_dec_replay_result(csil_root: &CsilCborValue) -> Result<ReplayResult, CsilCborError> {
    let requeued = {
        let csil_field = cbor_require(csil_root, "requeued")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    Ok(ReplayResult { requeued })
}

/// Encode a ReplayResult to canonical CSIL CBOR bytes.
pub fn encode_replay_result(csil_v: &ReplayResult) -> Vec<u8> {
    cbor_encode(&csil_enc_replay_result(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ReplayResult.
pub fn decode_replay_result(csil_data: &[u8]) -> Result<ReplayResult, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_replay_result(&csil_root)
}

/// Build the canonical CBOR value tree for a Credential.
fn csil_enc_credential(csil_v: &Credential) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(9);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("host"), cbor_text(&csil_v.host)));
    if let Some(csil_inner) = &csil_v.repo {
        csil_entries.push((cbor_text("repo"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("enabled"), cbor_bool(csil_v.enabled)));
    csil_entries.push((cbor_text("purpose"), cbor_text(&csil_v.purpose)));
    csil_entries.push((cbor_text("provider"), cbor_text(&csil_v.provider)));
    csil_entries.push((cbor_text("secret_ref"), cbor_text(&csil_v.secret_ref)));
    csil_entries.push((cbor_text("auth_method"), cbor_text(&csil_v.auth_method)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Credential from a decoded CBOR value tree.
fn csil_dec_credential(csil_root: &CsilCborValue) -> Result<Credential, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let repo = match cbor_map_get(csil_root, "repo") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let provider = {
        let csil_field = cbor_require(csil_root, "provider")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let purpose = {
        let csil_field = cbor_require(csil_root, "purpose")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let auth_method = {
        let csil_field = cbor_require(csil_root, "auth_method")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let host = {
        let csil_field = cbor_require(csil_root, "host")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let secret_ref = {
        let csil_field = cbor_require(csil_root, "secret_ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let enabled = {
        let csil_field = cbor_require(csil_root, "enabled")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(Credential {
        id,
        org,
        repo,
        provider,
        purpose,
        auth_method,
        host,
        secret_ref,
        enabled,
    })
}

/// Encode a Credential to canonical CSIL CBOR bytes.
pub fn encode_credential(csil_v: &Credential) -> Vec<u8> {
    cbor_encode(&csil_enc_credential(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Credential.
pub fn decode_credential(csil_data: &[u8]) -> Result<Credential, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_credential(&csil_root)
}

/// Build the canonical CBOR value tree for a SetCredentialRequest.
fn csil_enc_set_credential_request(csil_v: &SetCredentialRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(10);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    csil_entries.push((cbor_text("host"), cbor_text(&csil_v.host)));
    if let Some(csil_inner) = &csil_v.repo {
        csil_entries.push((cbor_text("repo"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.secret {
        csil_entries.push((cbor_text("secret"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.enabled {
        csil_entries.push((cbor_text("enabled"), cbor_bool(*csil_inner)));
    }
    csil_entries.push((cbor_text("purpose"), cbor_text(&csil_v.purpose)));
    csil_entries.push((cbor_text("provider"), cbor_text(&csil_v.provider)));
    csil_entries.push((cbor_text("secret_ref"), cbor_text(&csil_v.secret_ref)));
    csil_entries.push((cbor_text("auth_method"), cbor_text(&csil_v.auth_method)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SetCredentialRequest from a decoded CBOR value tree.
fn csil_dec_set_credential_request(
    csil_root: &CsilCborValue,
) -> Result<SetCredentialRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let repo = match cbor_map_get(csil_root, "repo") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let provider = {
        let csil_field = cbor_require(csil_root, "provider")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let purpose = {
        let csil_field = cbor_require(csil_root, "purpose")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let auth_method = {
        let csil_field = cbor_require(csil_root, "auth_method")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let host = {
        let csil_field = cbor_require(csil_root, "host")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let secret_ref = {
        let csil_field = cbor_require(csil_root, "secret_ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let secret = match cbor_map_get(csil_root, "secret") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let enabled = match cbor_map_get(csil_root, "enabled") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bool;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(SetCredentialRequest {
        ctx,
        org,
        repo,
        provider,
        purpose,
        auth_method,
        host,
        secret_ref,
        secret,
        enabled,
    })
}

/// Encode a SetCredentialRequest to canonical CSIL CBOR bytes.
pub fn encode_set_credential_request(csil_v: &SetCredentialRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_set_credential_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SetCredentialRequest.
pub fn decode_set_credential_request(
    csil_data: &[u8],
) -> Result<SetCredentialRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_set_credential_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListCredentialsRequest.
fn csil_enc_list_credentials_request(csil_v: &ListCredentialsRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("target"), csil_enc_org_repo_ref(&csil_v.target)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListCredentialsRequest from a decoded CBOR value tree.
fn csil_dec_list_credentials_request(
    csil_root: &CsilCborValue,
) -> Result<ListCredentialsRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let target = {
        let csil_field = cbor_require(csil_root, "target")?;
        let csil_decode = csil_dec_org_repo_ref;
        csil_decode(csil_field)?
    };
    Ok(ListCredentialsRequest { ctx, target })
}

/// Encode a ListCredentialsRequest to canonical CSIL CBOR bytes.
pub fn encode_list_credentials_request(csil_v: &ListCredentialsRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_list_credentials_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListCredentialsRequest.
pub fn decode_list_credentials_request(
    csil_data: &[u8],
) -> Result<ListCredentialsRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_credentials_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ListCredentialsResponse.
fn csil_enc_list_credentials_response(csil_v: &ListCredentialsResponse) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("credentials"),
        cbor_enc_array(&csil_v.credentials, csil_enc_credential),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ListCredentialsResponse from a decoded CBOR value tree.
fn csil_dec_list_credentials_response(
    csil_root: &CsilCborValue,
) -> Result<ListCredentialsResponse, CsilCborError> {
    let credentials = {
        let csil_field = cbor_require(csil_root, "credentials")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_credential);
        csil_decode(csil_field)?
    };
    Ok(ListCredentialsResponse { credentials })
}

/// Encode a ListCredentialsResponse to canonical CSIL CBOR bytes.
pub fn encode_list_credentials_response(csil_v: &ListCredentialsResponse) -> Vec<u8> {
    cbor_encode(&csil_enc_list_credentials_response(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ListCredentialsResponse.
pub fn decode_list_credentials_response(
    csil_data: &[u8],
) -> Result<ListCredentialsResponse, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_list_credentials_response(&csil_root)
}

/// Build the canonical CBOR value tree for a CredentialRef.
fn csil_enc_credential_ref(csil_v: &CredentialRef) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(2);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("credential_id"), cbor_text(&csil_v.credential_id)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CredentialRef from a decoded CBOR value tree.
fn csil_dec_credential_ref(csil_root: &CsilCborValue) -> Result<CredentialRef, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let credential_id = {
        let csil_field = cbor_require(csil_root, "credential_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(CredentialRef { ctx, credential_id })
}

/// Encode a CredentialRef to canonical CSIL CBOR bytes.
pub fn encode_credential_ref(csil_v: &CredentialRef) -> Vec<u8> {
    cbor_encode(&csil_enc_credential_ref(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CredentialRef.
pub fn decode_credential_ref(csil_data: &[u8]) -> Result<CredentialRef, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_credential_ref(&csil_root)
}

/// Build the canonical CBOR value tree for a ConfigureMirrorRequest.
fn csil_enc_configure_mirror_request(csil_v: &ConfigureMirrorRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    if let Some(csil_inner) = &csil_v.mode {
        csil_entries.push((cbor_text("mode"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.enabled {
        csil_entries.push((cbor_text("enabled"), cbor_bool(*csil_inner)));
    }
    csil_entries.push((cbor_text("github_repo"), cbor_text(&csil_v.github_repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ConfigureMirrorRequest from a decoded CBOR value tree.
fn csil_dec_configure_mirror_request(
    csil_root: &CsilCborValue,
) -> Result<ConfigureMirrorRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let github_repo = {
        let csil_field = cbor_require(csil_root, "github_repo")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let mode = match cbor_map_get(csil_root, "mode") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let enabled = match cbor_map_get(csil_root, "enabled") {
        Some(csil_field) => {
            let csil_decode = cbor_as_bool;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ConfigureMirrorRequest {
        ctx,
        repo,
        github_repo,
        mode,
        enabled,
    })
}

/// Encode a ConfigureMirrorRequest to canonical CSIL CBOR bytes.
pub fn encode_configure_mirror_request(csil_v: &ConfigureMirrorRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_configure_mirror_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ConfigureMirrorRequest.
pub fn decode_configure_mirror_request(
    csil_data: &[u8],
) -> Result<ConfigureMirrorRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_configure_mirror_request(&csil_root)
}

/// Build the canonical CBOR value tree for a ReconcileReport.
fn csil_enc_reconcile_report(csil_v: &ReconcileReport) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(7);
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    if let Some(csil_inner) = &csil_v.error {
        csil_entries.push((cbor_text("error"), cbor_text(csil_inner)));
    }
    csil_entries.push((
        cbor_text("drift_refs"),
        cbor_enc_array(&csil_v.drift_refs, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((cbor_text("started_at"), cbor_int(csil_v.started_at)));
    csil_entries.push((cbor_text("finished_at"), cbor_int(csil_v.finished_at)));
    csil_entries.push((cbor_text("refs_pushed"), cbor_uint(csil_v.refs_pushed)));
    csil_entries.push((cbor_text("refs_checked"), cbor_uint(csil_v.refs_checked)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ReconcileReport from a decoded CBOR value tree.
fn csil_dec_reconcile_report(csil_root: &CsilCborValue) -> Result<ReconcileReport, CsilCborError> {
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let started_at = {
        let csil_field = cbor_require(csil_root, "started_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let finished_at = {
        let csil_field = cbor_require(csil_root, "finished_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let refs_checked = {
        let csil_field = cbor_require(csil_root, "refs_checked")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let refs_pushed = {
        let csil_field = cbor_require(csil_root, "refs_pushed")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let drift_refs = {
        let csil_field = cbor_require(csil_root, "drift_refs")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let error = match cbor_map_get(csil_root, "error") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ReconcileReport {
        repo,
        started_at,
        finished_at,
        refs_checked,
        refs_pushed,
        drift_refs,
        error,
    })
}

/// Encode a ReconcileReport to canonical CSIL CBOR bytes.
pub fn encode_reconcile_report(csil_v: &ReconcileReport) -> Vec<u8> {
    cbor_encode(&csil_enc_reconcile_report(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ReconcileReport.
pub fn decode_reconcile_report(csil_data: &[u8]) -> Result<ReconcileReport, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_reconcile_report(&csil_root)
}

/// Build the canonical CBOR value tree for a ImportRequest.
fn csil_enc_import_request(csil_v: &ImportRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("org"), cbor_text(&csil_v.org)));
    if let Some(csil_inner) = &csil_v.mode {
        csil_entries.push((cbor_text("mode"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    if let Some(csil_inner) = &csil_v.visibility {
        csil_entries.push((cbor_text("visibility"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("github_repo"), cbor_text(&csil_v.github_repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ImportRequest from a decoded CBOR value tree.
fn csil_dec_import_request(csil_root: &CsilCborValue) -> Result<ImportRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let org = {
        let csil_field = cbor_require(csil_root, "org")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let github_repo = {
        let csil_field = cbor_require(csil_root, "github_repo")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let mode = match cbor_map_get(csil_root, "mode") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let visibility = match cbor_map_get(csil_root, "visibility") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(ImportRequest {
        ctx,
        org,
        name,
        github_repo,
        mode,
        visibility,
    })
}

/// Encode a ImportRequest to canonical CSIL CBOR bytes.
pub fn encode_import_request(csil_v: &ImportRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_import_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ImportRequest.
pub fn decode_import_request(csil_data: &[u8]) -> Result<ImportRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_import_request(&csil_root)
}

/// Build the canonical CBOR value tree for a FedHello.
fn csil_enc_fed_hello(csil_v: &FedHello) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(9);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("nonce"), cbor_bytes(&csil_v.nonce)));
    csil_entries.push((cbor_text("key_id"), cbor_text(&csil_v.key_id)));
    csil_entries.push((cbor_text("instance"), cbor_text(&csil_v.instance)));
    csil_entries.push((cbor_text("signature"), cbor_bytes(&csil_v.signature)));
    csil_entries.push((cbor_text("timestamp"), cbor_int(csil_v.timestamp)));
    csil_entries.push((cbor_text("instance_id"), cbor_text(&csil_v.instance_id)));
    csil_entries.push((
        cbor_text("capabilities"),
        csil_enc_fed_capabilities(&csil_v.capabilities),
    ));
    csil_entries.push((
        cbor_text("instance_identity"),
        cbor_text(&csil_v.instance_identity),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a FedHello from a decoded CBOR value tree.
fn csil_dec_fed_hello(csil_root: &CsilCborValue) -> Result<FedHello, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let instance = {
        let csil_field = cbor_require(csil_root, "instance")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let instance_identity = {
        let csil_field = cbor_require(csil_root, "instance_identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let instance_id = {
        let csil_field = cbor_require(csil_root, "instance_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let key_id = {
        let csil_field = cbor_require(csil_root, "key_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let nonce = {
        let csil_field = cbor_require(csil_root, "nonce")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let timestamp = {
        let csil_field = cbor_require(csil_root, "timestamp")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let signature = {
        let csil_field = cbor_require(csil_root, "signature")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let capabilities = {
        let csil_field = cbor_require(csil_root, "capabilities")?;
        let csil_decode = csil_dec_fed_capabilities;
        csil_decode(csil_field)?
    };
    Ok(FedHello {
        ctx,
        instance,
        instance_identity,
        instance_id,
        key_id,
        nonce,
        timestamp,
        signature,
        capabilities,
    })
}

/// Encode a FedHello to canonical CSIL CBOR bytes.
pub fn encode_fed_hello(csil_v: &FedHello) -> Vec<u8> {
    cbor_encode(&csil_enc_fed_hello(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a FedHello.
pub fn decode_fed_hello(csil_data: &[u8]) -> Result<FedHello, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_fed_hello(&csil_root)
}

/// Build the canonical CBOR value tree for a FedCapabilities.
fn csil_enc_fed_capabilities(csil_v: &FedCapabilities) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(4);
    csil_entries.push((
        cbor_text("vcs_types"),
        cbor_enc_array(&csil_v.vcs_types, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((
        cbor_text("event_kinds"),
        cbor_enc_array(&csil_v.event_kinds, |csil_elem| cbor_text(csil_elem)),
    ));
    csil_entries.push((
        cbor_text("max_frame_bytes"),
        cbor_uint(csil_v.max_frame_bytes),
    ));
    csil_entries.push((
        cbor_text("protocol_versions"),
        cbor_enc_array(&csil_v.protocol_versions, |csil_elem| cbor_text(csil_elem)),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a FedCapabilities from a decoded CBOR value tree.
fn csil_dec_fed_capabilities(csil_root: &CsilCborValue) -> Result<FedCapabilities, CsilCborError> {
    let protocol_versions = {
        let csil_field = cbor_require(csil_root, "protocol_versions")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let vcs_types = {
        let csil_field = cbor_require(csil_root, "vcs_types")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let event_kinds = {
        let csil_field = cbor_require(csil_root, "event_kinds")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let max_frame_bytes = {
        let csil_field = cbor_require(csil_root, "max_frame_bytes")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    Ok(FedCapabilities {
        protocol_versions,
        vcs_types,
        event_kinds,
        max_frame_bytes,
    })
}

/// Encode a FedCapabilities to canonical CSIL CBOR bytes.
pub fn encode_fed_capabilities(csil_v: &FedCapabilities) -> Vec<u8> {
    cbor_encode(&csil_enc_fed_capabilities(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a FedCapabilities.
pub fn decode_fed_capabilities(csil_data: &[u8]) -> Result<FedCapabilities, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_fed_capabilities(&csil_root)
}

/// Build the canonical CBOR value tree for a FedHelloAck.
fn csil_enc_fed_hello_ack(csil_v: &FedHelloAck) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(10);
    csil_entries.push((cbor_text("nonce"), cbor_bytes(&csil_v.nonce)));
    csil_entries.push((cbor_text("key_id"), cbor_text(&csil_v.key_id)));
    csil_entries.push((cbor_text("instance"), cbor_text(&csil_v.instance)));
    csil_entries.push((cbor_text("signature"), cbor_bytes(&csil_v.signature)));
    csil_entries.push((cbor_text("timestamp"), cbor_int(csil_v.timestamp)));
    csil_entries.push((cbor_text("expires_at"), cbor_int(csil_v.expires_at)));
    csil_entries.push((cbor_text("instance_id"), cbor_text(&csil_v.instance_id)));
    csil_entries.push((
        cbor_text("capabilities"),
        csil_enc_fed_capabilities(&csil_v.capabilities),
    ));
    csil_entries.push((
        cbor_text("instance_identity"),
        cbor_text(&csil_v.instance_identity),
    ));
    csil_entries.push((
        cbor_text("peer_session_token"),
        cbor_text(&csil_v.peer_session_token),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a FedHelloAck from a decoded CBOR value tree.
fn csil_dec_fed_hello_ack(csil_root: &CsilCborValue) -> Result<FedHelloAck, CsilCborError> {
    let instance = {
        let csil_field = cbor_require(csil_root, "instance")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let instance_identity = {
        let csil_field = cbor_require(csil_root, "instance_identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let instance_id = {
        let csil_field = cbor_require(csil_root, "instance_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let key_id = {
        let csil_field = cbor_require(csil_root, "key_id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let nonce = {
        let csil_field = cbor_require(csil_root, "nonce")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let timestamp = {
        let csil_field = cbor_require(csil_root, "timestamp")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let signature = {
        let csil_field = cbor_require(csil_root, "signature")?;
        let csil_decode = cbor_as_bytes;
        csil_decode(csil_field)?
    };
    let capabilities = {
        let csil_field = cbor_require(csil_root, "capabilities")?;
        let csil_decode = csil_dec_fed_capabilities;
        csil_decode(csil_field)?
    };
    let peer_session_token = {
        let csil_field = cbor_require(csil_root, "peer_session_token")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let expires_at = {
        let csil_field = cbor_require(csil_root, "expires_at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(FedHelloAck {
        instance,
        instance_identity,
        instance_id,
        key_id,
        nonce,
        timestamp,
        signature,
        capabilities,
        peer_session_token,
        expires_at,
    })
}

/// Encode a FedHelloAck to canonical CSIL CBOR bytes.
pub fn encode_fed_hello_ack(csil_v: &FedHelloAck) -> Vec<u8> {
    cbor_encode(&csil_enc_fed_hello_ack(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a FedHelloAck.
pub fn decode_fed_hello_ack(csil_data: &[u8]) -> Result<FedHelloAck, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_fed_hello_ack(&csil_root)
}

/// Build the canonical CBOR value tree for a VerifyHeadRequest.
fn csil_enc_verify_head_request(csil_v: &VerifyHeadRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("ref"), cbor_text(&csil_v.r#ref)));
    csil_entries.push((cbor_text("sha"), cbor_text(&csil_v.sha)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("on_behalf_of"), cbor_text(&csil_v.on_behalf_of)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a VerifyHeadRequest from a decoded CBOR value tree.
fn csil_dec_verify_head_request(
    csil_root: &CsilCborValue,
) -> Result<VerifyHeadRequest, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let r#ref = {
        let csil_field = cbor_require(csil_root, "ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let sha = {
        let csil_field = cbor_require(csil_root, "sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let on_behalf_of = {
        let csil_field = cbor_require(csil_root, "on_behalf_of")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(VerifyHeadRequest {
        ctx,
        repo,
        r#ref,
        sha,
        on_behalf_of,
    })
}

/// Encode a VerifyHeadRequest to canonical CSIL CBOR bytes.
pub fn encode_verify_head_request(csil_v: &VerifyHeadRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_verify_head_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a VerifyHeadRequest.
pub fn decode_verify_head_request(csil_data: &[u8]) -> Result<VerifyHeadRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_verify_head_request(&csil_root)
}

/// Build the canonical CBOR value tree for a HeadQuery.
fn csil_enc_head_query(csil_v: &HeadQuery) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ctx"), csil_enc_ctx(&csil_v.ctx)));
    csil_entries.push((cbor_text("ref"), cbor_text(&csil_v.r#ref)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a HeadQuery from a decoded CBOR value tree.
fn csil_dec_head_query(csil_root: &CsilCborValue) -> Result<HeadQuery, CsilCborError> {
    let ctx = {
        let csil_field = cbor_require(csil_root, "ctx")?;
        let csil_decode = csil_dec_ctx;
        csil_decode(csil_field)?
    };
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let r#ref = {
        let csil_field = cbor_require(csil_root, "ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(HeadQuery { ctx, repo, r#ref })
}

/// Encode a HeadQuery to canonical CSIL CBOR bytes.
pub fn encode_head_query(csil_v: &HeadQuery) -> Vec<u8> {
    cbor_encode(&csil_enc_head_query(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a HeadQuery.
pub fn decode_head_query(csil_data: &[u8]) -> Result<HeadQuery, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_head_query(&csil_root)
}

/// Build the canonical CBOR value tree for a HeadInfo.
fn csil_enc_head_info(csil_v: &HeadInfo) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(6);
    csil_entries.push((cbor_text("ref"), cbor_text(&csil_v.r#ref)));
    csil_entries.push((cbor_text("sha"), cbor_text(&csil_v.sha)));
    csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(&csil_v.repo)));
    csil_entries.push((cbor_text("as_of"), cbor_int(csil_v.as_of)));
    csil_entries.push((cbor_text("exists"), cbor_bool(csil_v.exists)));
    csil_entries.push((cbor_text("readable"), cbor_bool(csil_v.readable)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a HeadInfo from a decoded CBOR value tree.
fn csil_dec_head_info(csil_root: &CsilCborValue) -> Result<HeadInfo, CsilCborError> {
    let repo = {
        let csil_field = cbor_require(csil_root, "repo")?;
        let csil_decode = csil_dec_repo_ref;
        csil_decode(csil_field)?
    };
    let r#ref = {
        let csil_field = cbor_require(csil_root, "ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let sha = {
        let csil_field = cbor_require(csil_root, "sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let exists = {
        let csil_field = cbor_require(csil_root, "exists")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let readable = {
        let csil_field = cbor_require(csil_root, "readable")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let as_of = {
        let csil_field = cbor_require(csil_root, "as_of")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    Ok(HeadInfo {
        repo,
        r#ref,
        sha,
        exists,
        readable,
        as_of,
    })
}

/// Encode a HeadInfo to canonical CSIL CBOR bytes.
pub fn encode_head_info(csil_v: &HeadInfo) -> Vec<u8> {
    cbor_encode(&csil_enc_head_info(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a HeadInfo.
pub fn decode_head_info(csil_data: &[u8]) -> Result<HeadInfo, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_head_info(&csil_root)
}

/// Build the canonical CBOR value tree for a CommitSummary.
fn csil_enc_commit_summary(csil_v: &CommitSummary) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("id"), cbor_text(&csil_v.id)));
    csil_entries.push((cbor_text("signature"), cbor_text(&csil_v.signature)));
    csil_entries.push((cbor_text("author_name"), cbor_text(&csil_v.author_name)));
    csil_entries.push((cbor_text("author_email"), cbor_text(&csil_v.author_email)));
    csil_entries.push((cbor_text("message_head"), cbor_text(&csil_v.message_head)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a CommitSummary from a decoded CBOR value tree.
fn csil_dec_commit_summary(csil_root: &CsilCborValue) -> Result<CommitSummary, CsilCborError> {
    let id = {
        let csil_field = cbor_require(csil_root, "id")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let author_name = {
        let csil_field = cbor_require(csil_root, "author_name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let author_email = {
        let csil_field = cbor_require(csil_root, "author_email")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let message_head = {
        let csil_field = cbor_require(csil_root, "message_head")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let signature = {
        let csil_field = cbor_require(csil_root, "signature")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(CommitSummary {
        id,
        author_name,
        author_email,
        message_head,
        signature,
    })
}

/// Encode a CommitSummary to canonical CSIL CBOR bytes.
pub fn encode_commit_summary(csil_v: &CommitSummary) -> Vec<u8> {
    cbor_encode(&csil_enc_commit_summary(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a CommitSummary.
pub fn decode_commit_summary(csil_data: &[u8]) -> Result<CommitSummary, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_commit_summary(&csil_root)
}

/// Build the canonical CBOR value tree for a PushPayload.
fn csil_enc_push_payload(csil_v: &PushPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("ref"), cbor_text(&csil_v.r#ref)));
    csil_entries.push((cbor_text("after"), cbor_text(&csil_v.after)));
    csil_entries.push((cbor_text("before"), cbor_text(&csil_v.before)));
    csil_entries.push((cbor_text("forced"), cbor_bool(csil_v.forced)));
    csil_entries.push((
        cbor_text("commits"),
        cbor_enc_array(&csil_v.commits, csil_enc_commit_summary),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a PushPayload from a decoded CBOR value tree.
fn csil_dec_push_payload(csil_root: &CsilCborValue) -> Result<PushPayload, CsilCborError> {
    let r#ref = {
        let csil_field = cbor_require(csil_root, "ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let before = {
        let csil_field = cbor_require(csil_root, "before")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let after = {
        let csil_field = cbor_require(csil_root, "after")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let forced = {
        let csil_field = cbor_require(csil_root, "forced")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let commits = {
        let csil_field = cbor_require(csil_root, "commits")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, csil_dec_commit_summary);
        csil_decode(csil_field)?
    };
    Ok(PushPayload {
        r#ref,
        before,
        after,
        forced,
        commits,
    })
}

/// Encode a PushPayload to canonical CSIL CBOR bytes.
pub fn encode_push_payload(csil_v: &PushPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_push_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a PushPayload.
pub fn decode_push_payload(csil_data: &[u8]) -> Result<PushPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_push_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a RefPayload.
fn csil_enc_ref_payload(csil_v: &RefPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("ref"), cbor_text(&csil_v.r#ref)));
    if let Some(csil_inner) = &csil_v.after {
        csil_entries.push((cbor_text("after"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.before {
        csil_entries.push((cbor_text("before"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a RefPayload from a decoded CBOR value tree.
fn csil_dec_ref_payload(csil_root: &CsilCborValue) -> Result<RefPayload, CsilCborError> {
    let r#ref = {
        let csil_field = cbor_require(csil_root, "ref")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let before = match cbor_map_get(csil_root, "before") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let after = match cbor_map_get(csil_root, "after") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(RefPayload {
        r#ref,
        before,
        after,
    })
}

/// Encode a RefPayload to canonical CSIL CBOR bytes.
pub fn encode_ref_payload(csil_v: &RefPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_ref_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a RefPayload.
pub fn decode_ref_payload(csil_data: &[u8]) -> Result<RefPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_ref_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a PrPayload.
fn csil_enc_pr_payload(csil_v: &PrPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    if let Some(csil_inner) = &csil_v.state {
        csil_entries.push((cbor_text("state"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("number"), cbor_uint(csil_v.number)));
    if let Some(csil_inner) = &csil_v.base_ref {
        csil_entries.push((cbor_text("base_ref"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.head_sha {
        csil_entries.push((cbor_text("head_sha"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.merge_kind {
        csil_entries.push((cbor_text("merge_kind"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a PrPayload from a decoded CBOR value tree.
fn csil_dec_pr_payload(csil_root: &CsilCborValue) -> Result<PrPayload, CsilCborError> {
    let number = {
        let csil_field = cbor_require(csil_root, "number")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let head_sha = match cbor_map_get(csil_root, "head_sha") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let base_ref = match cbor_map_get(csil_root, "base_ref") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let state = match cbor_map_get(csil_root, "state") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let merge_kind = match cbor_map_get(csil_root, "merge_kind") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(PrPayload {
        number,
        head_sha,
        base_ref,
        state,
        merge_kind,
    })
}

/// Encode a PrPayload to canonical CSIL CBOR bytes.
pub fn encode_pr_payload(csil_v: &PrPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_pr_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a PrPayload.
pub fn decode_pr_payload(csil_data: &[u8]) -> Result<PrPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_pr_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a ReleasePayload.
fn csil_enc_release_payload(csil_v: &ReleasePayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(5);
    csil_entries.push((cbor_text("tag"), cbor_text(&csil_v.tag)));
    if let Some(csil_inner) = &csil_v.body {
        csil_entries.push((cbor_text("body"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("name"), cbor_text(&csil_v.name)));
    csil_entries.push((cbor_text("draft"), cbor_bool(csil_v.draft)));
    csil_entries.push((cbor_text("prerelease"), cbor_bool(csil_v.prerelease)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a ReleasePayload from a decoded CBOR value tree.
fn csil_dec_release_payload(csil_root: &CsilCborValue) -> Result<ReleasePayload, CsilCborError> {
    let tag = {
        let csil_field = cbor_require(csil_root, "tag")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let name = {
        let csil_field = cbor_require(csil_root, "name")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let body = match cbor_map_get(csil_root, "body") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let draft = {
        let csil_field = cbor_require(csil_root, "draft")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    let prerelease = {
        let csil_field = cbor_require(csil_root, "prerelease")?;
        let csil_decode = cbor_as_bool;
        csil_decode(csil_field)?
    };
    Ok(ReleasePayload {
        tag,
        name,
        body,
        draft,
        prerelease,
    })
}

/// Encode a ReleasePayload to canonical CSIL CBOR bytes.
pub fn encode_release_payload(csil_v: &ReleasePayload) -> Vec<u8> {
    cbor_encode(&csil_enc_release_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a ReleasePayload.
pub fn decode_release_payload(csil_data: &[u8]) -> Result<ReleasePayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_release_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a MirrorPayload.
fn csil_enc_mirror_payload(csil_v: &MirrorPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    if let Some(csil_inner) = &csil_v.error {
        csil_entries.push((cbor_text("error"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("object_key"), cbor_text(&csil_v.object_key)));
    csil_entries.push((cbor_text("action_kind"), cbor_text(&csil_v.action_kind)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a MirrorPayload from a decoded CBOR value tree.
fn csil_dec_mirror_payload(csil_root: &CsilCborValue) -> Result<MirrorPayload, CsilCborError> {
    let object_key = {
        let csil_field = cbor_require(csil_root, "object_key")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let error = match cbor_map_get(csil_root, "error") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let action_kind = {
        let csil_field = cbor_require(csil_root, "action_kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(MirrorPayload {
        object_key,
        error,
        action_kind,
    })
}

/// Encode a MirrorPayload to canonical CSIL CBOR bytes.
pub fn encode_mirror_payload(csil_v: &MirrorPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_mirror_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a MirrorPayload.
pub fn decode_mirror_payload(csil_data: &[u8]) -> Result<MirrorPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_mirror_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a SettingsPayload.
fn csil_enc_settings_payload(csil_v: &SettingsPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("changed"),
        cbor_enc_array(&csil_v.changed, |csil_elem| cbor_text(csil_elem)),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SettingsPayload from a decoded CBOR value tree.
fn csil_dec_settings_payload(csil_root: &CsilCborValue) -> Result<SettingsPayload, CsilCborError> {
    let changed = {
        let csil_field = cbor_require(csil_root, "changed")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(SettingsPayload { changed })
}

/// Encode a SettingsPayload to canonical CSIL CBOR bytes.
pub fn encode_settings_payload(csil_v: &SettingsPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_settings_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SettingsPayload.
pub fn decode_settings_payload(csil_data: &[u8]) -> Result<SettingsPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_settings_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a StatusPayload.
fn csil_enc_status_payload(csil_v: &StatusPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    csil_entries.push((cbor_text("sha"), cbor_text(&csil_v.sha)));
    csil_entries.push((cbor_text("state"), cbor_text(&csil_v.state)));
    csil_entries.push((cbor_text("context"), cbor_text(&csil_v.context)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a StatusPayload from a decoded CBOR value tree.
fn csil_dec_status_payload(csil_root: &CsilCborValue) -> Result<StatusPayload, CsilCborError> {
    let sha = {
        let csil_field = cbor_require(csil_root, "sha")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let context = {
        let csil_field = cbor_require(csil_root, "context")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let state = {
        let csil_field = cbor_require(csil_root, "state")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    Ok(StatusPayload {
        sha,
        context,
        state,
    })
}

/// Encode a StatusPayload to canonical CSIL CBOR bytes.
pub fn encode_status_payload(csil_v: &StatusPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_status_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a StatusPayload.
pub fn decode_status_payload(csil_data: &[u8]) -> Result<StatusPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_status_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a MembershipPayload.
fn csil_enc_membership_payload(csil_v: &MembershipPayload) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    if let Some(csil_inner) = &csil_v.role {
        csil_entries.push((cbor_text("role"), cbor_text(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.team {
        csil_entries.push((cbor_text("team"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("identity"), cbor_text(&csil_v.identity)));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a MembershipPayload from a decoded CBOR value tree.
fn csil_dec_membership_payload(
    csil_root: &CsilCborValue,
) -> Result<MembershipPayload, CsilCborError> {
    let identity = {
        let csil_field = cbor_require(csil_root, "identity")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let role = match cbor_map_get(csil_root, "role") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let team = match cbor_map_get(csil_root, "team") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(MembershipPayload {
        identity,
        role,
        team,
    })
}

/// Encode a MembershipPayload to canonical CSIL CBOR bytes.
pub fn encode_membership_payload(csil_v: &MembershipPayload) -> Vec<u8> {
    cbor_encode(&csil_enc_membership_payload(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a MembershipPayload.
pub fn decode_membership_payload(csil_data: &[u8]) -> Result<MembershipPayload, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_membership_payload(&csil_root)
}

/// Build the canonical CBOR value tree for a Event.
fn csil_enc_event(csil_v: &Event) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(9);
    csil_entries.push((cbor_text("at"), cbor_int(csil_v.at)));
    if let Some(csil_inner) = &csil_v.org {
        csil_entries.push((cbor_text("org"), cbor_text(csil_inner)));
    }
    csil_entries.push((cbor_text("seq"), cbor_uint(csil_v.seq)));
    csil_entries.push((cbor_text("kind"), cbor_text(&csil_v.kind)));
    csil_entries.push((cbor_text("lane"), cbor_text(&csil_v.lane)));
    if let Some(csil_inner) = &csil_v.repo {
        csil_entries.push((cbor_text("repo"), csil_enc_repo_ref(csil_inner)));
    }
    if let Some(csil_inner) = &csil_v.actor {
        csil_entries.push((cbor_text("actor"), csil_enc_identity(csil_inner)));
    }
    csil_entries.push((
        cbor_text("payload"),
        csil_enc_event_payload(&csil_v.payload),
    ));
    if let Some(csil_inner) = &csil_v.subject {
        csil_entries.push((cbor_text("subject"), cbor_text(csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a Event from a decoded CBOR value tree.
fn csil_dec_event(csil_root: &CsilCborValue) -> Result<Event, CsilCborError> {
    let lane = {
        let csil_field = cbor_require(csil_root, "lane")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let seq = {
        let csil_field = cbor_require(csil_root, "seq")?;
        let csil_decode = cbor_as_u64;
        csil_decode(csil_field)?
    };
    let kind = {
        let csil_field = cbor_require(csil_root, "kind")?;
        let csil_decode = cbor_as_text;
        csil_decode(csil_field)?
    };
    let actor = match cbor_map_get(csil_root, "actor") {
        Some(csil_field) => {
            let csil_decode = csil_dec_identity;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let repo = match cbor_map_get(csil_root, "repo") {
        Some(csil_field) => {
            let csil_decode = csil_dec_repo_ref;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let org = match cbor_map_get(csil_root, "org") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let subject = match cbor_map_get(csil_root, "subject") {
        Some(csil_field) => {
            let csil_decode = cbor_as_text;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let at = {
        let csil_field = cbor_require(csil_root, "at")?;
        let csil_decode = cbor_as_i64;
        csil_decode(csil_field)?
    };
    let payload = {
        let csil_field = cbor_require(csil_root, "payload")?;
        let csil_decode = csil_dec_event_payload;
        csil_decode(csil_field)?
    };
    Ok(Event {
        lane,
        seq,
        kind,
        actor,
        repo,
        org,
        subject,
        at,
        payload,
    })
}

/// Encode a Event to canonical CSIL CBOR bytes.
pub fn encode_event(csil_v: &Event) -> Vec<u8> {
    cbor_encode(&csil_enc_event(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a Event.
pub fn decode_event(csil_data: &[u8]) -> Result<Event, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_event(&csil_root)
}

/// Build the canonical CBOR value tree for a SubscribeRequest.
fn csil_enc_subscribe_request(csil_v: &SubscribeRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(3);
    if let Some(csil_inner) = &csil_v.kinds {
        csil_entries.push((
            cbor_text("kinds"),
            cbor_enc_array(csil_inner, |csil_elem| cbor_text(csil_elem)),
        ));
    }
    csil_entries.push((
        cbor_text("lanes"),
        cbor_enc_array(&csil_v.lanes, |csil_elem| cbor_text(csil_elem)),
    ));
    if let Some(csil_inner) = &csil_v.after_seq {
        csil_entries.push((cbor_text("after_seq"), cbor_uint(*csil_inner)));
    }
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a SubscribeRequest from a decoded CBOR value tree.
fn csil_dec_subscribe_request(
    csil_root: &CsilCborValue,
) -> Result<SubscribeRequest, CsilCborError> {
    let lanes = {
        let csil_field = cbor_require(csil_root, "lanes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    let kinds = match cbor_map_get(csil_root, "kinds") {
        Some(csil_field) => {
            let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    let after_seq = match cbor_map_get(csil_root, "after_seq") {
        Some(csil_field) => {
            let csil_decode = cbor_as_u64;
            Some(csil_decode(csil_field)?)
        }
        None => None,
    };
    Ok(SubscribeRequest {
        lanes,
        kinds,
        after_seq,
    })
}

/// Encode a SubscribeRequest to canonical CSIL CBOR bytes.
pub fn encode_subscribe_request(csil_v: &SubscribeRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_subscribe_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a SubscribeRequest.
pub fn decode_subscribe_request(csil_data: &[u8]) -> Result<SubscribeRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_subscribe_request(&csil_root)
}

/// Build the canonical CBOR value tree for a UnsubscribeRequest.
fn csil_enc_unsubscribe_request(csil_v: &UnsubscribeRequest) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("lanes"),
        cbor_enc_array(&csil_v.lanes, |csil_elem| cbor_text(csil_elem)),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UnsubscribeRequest from a decoded CBOR value tree.
fn csil_dec_unsubscribe_request(
    csil_root: &CsilCborValue,
) -> Result<UnsubscribeRequest, CsilCborError> {
    let lanes = {
        let csil_field = cbor_require(csil_root, "lanes")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(UnsubscribeRequest { lanes })
}

/// Encode a UnsubscribeRequest to canonical CSIL CBOR bytes.
pub fn encode_unsubscribe_request(csil_v: &UnsubscribeRequest) -> Vec<u8> {
    cbor_encode(&csil_enc_unsubscribe_request(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UnsubscribeRequest.
pub fn decode_unsubscribe_request(csil_data: &[u8]) -> Result<UnsubscribeRequest, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_unsubscribe_request(&csil_root)
}

/// Build the canonical CBOR value tree for a UnsribeMarker.
fn csil_enc_unsribe_marker(csil_v: &UnsribeMarker) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("unsubscribe"),
        csil_enc_unsubscribe_request(&csil_v.unsubscribe),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a UnsribeMarker from a decoded CBOR value tree.
fn csil_dec_unsribe_marker(csil_root: &CsilCborValue) -> Result<UnsribeMarker, CsilCborError> {
    let unsubscribe = {
        let csil_field = cbor_require(csil_root, "unsubscribe")?;
        let csil_decode = csil_dec_unsubscribe_request;
        csil_decode(csil_field)?
    };
    Ok(UnsribeMarker { unsubscribe })
}

/// Encode a UnsribeMarker to canonical CSIL CBOR bytes.
pub fn encode_unsribe_marker(csil_v: &UnsribeMarker) -> Vec<u8> {
    cbor_encode(&csil_enc_unsribe_marker(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a UnsribeMarker.
pub fn decode_unsribe_marker(csil_data: &[u8]) -> Result<UnsribeMarker, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_unsribe_marker(&csil_root)
}

/// Build the canonical CBOR value tree for a StreamAck.
fn csil_enc_stream_ack(csil_v: &StreamAck) -> CsilCborValue {
    let mut csil_entries: Vec<(CsilCborValue, CsilCborValue)> = Vec::with_capacity(1);
    csil_entries.push((
        cbor_text("subscribed"),
        cbor_enc_array(&csil_v.subscribed, |csil_elem| cbor_text(csil_elem)),
    ));
    CsilCborValue::Map(csil_entries)
}

/// Reconstruct a StreamAck from a decoded CBOR value tree.
fn csil_dec_stream_ack(csil_root: &CsilCborValue) -> Result<StreamAck, CsilCborError> {
    let subscribed = {
        let csil_field = cbor_require(csil_root, "subscribed")?;
        let csil_decode = |csil_v| cbor_dec_array(csil_v, cbor_as_text);
        csil_decode(csil_field)?
    };
    Ok(StreamAck { subscribed })
}

/// Encode a StreamAck to canonical CSIL CBOR bytes.
pub fn encode_stream_ack(csil_v: &StreamAck) -> Vec<u8> {
    cbor_encode(&csil_enc_stream_ack(csil_v))
}

/// Decode canonical CSIL CBOR bytes into a StreamAck.
pub fn decode_stream_ack(csil_data: &[u8]) -> Result<StreamAck, CsilCborError> {
    let csil_root = cbor_decode(csil_data)?;
    csil_dec_stream_ack(&csil_root)
}

/// Encode a EventPayload union as a tagged sum `[variant_index, value]`.
fn csil_enc_event_payload(csil_v: &EventPayload) -> CsilCborValue {
    match csil_v {
        EventPayload::Variant0(csil_x) => {
            CsilCborValue::Array(vec![CsilCborValue::Uint(0), csil_enc_push_payload(csil_x)])
        }
        EventPayload::Variant1(csil_x) => {
            CsilCborValue::Array(vec![CsilCborValue::Uint(1), csil_enc_ref_payload(csil_x)])
        }
        EventPayload::Variant2(csil_x) => {
            CsilCborValue::Array(vec![CsilCborValue::Uint(2), csil_enc_pr_payload(csil_x)])
        }
        EventPayload::Variant3(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(3),
            csil_enc_release_payload(csil_x),
        ]),
        EventPayload::Variant4(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(4),
            csil_enc_mirror_payload(csil_x),
        ]),
        EventPayload::Variant5(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(5),
            csil_enc_settings_payload(csil_x),
        ]),
        EventPayload::Variant6(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(6),
            csil_enc_status_payload(csil_x),
        ]),
        EventPayload::Variant7(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(7),
            csil_enc_membership_payload(csil_x),
        ]),
    }
}

/// Decode a tagged sum `[variant_index, value]` into a EventPayload union.
fn csil_dec_event_payload(csil_v: &CsilCborValue) -> Result<EventPayload, CsilCborError> {
    let csil_arr = match csil_v {
        CsilCborValue::Array(csil_a) => csil_a,
        _ => {
            return Err(CsilCborError(
                "csil cbor: union expects a 2-element array".to_string(),
            ))
        }
    };
    if csil_arr.len() != 2 {
        return Err(CsilCborError(format!(
            "csil cbor: union array has {} elements, expected 2",
            csil_arr.len()
        )));
    }
    let csil_idx = cbor_as_u64(&csil_arr[0])?;
    match csil_idx {
        0 => {
            let csil_decode = csil_dec_push_payload;
            Ok(EventPayload::Variant0(csil_decode(&csil_arr[1])?))
        }
        1 => {
            let csil_decode = csil_dec_ref_payload;
            Ok(EventPayload::Variant1(csil_decode(&csil_arr[1])?))
        }
        2 => {
            let csil_decode = csil_dec_pr_payload;
            Ok(EventPayload::Variant2(csil_decode(&csil_arr[1])?))
        }
        3 => {
            let csil_decode = csil_dec_release_payload;
            Ok(EventPayload::Variant3(csil_decode(&csil_arr[1])?))
        }
        4 => {
            let csil_decode = csil_dec_mirror_payload;
            Ok(EventPayload::Variant4(csil_decode(&csil_arr[1])?))
        }
        5 => {
            let csil_decode = csil_dec_settings_payload;
            Ok(EventPayload::Variant5(csil_decode(&csil_arr[1])?))
        }
        6 => {
            let csil_decode = csil_dec_status_payload;
            Ok(EventPayload::Variant6(csil_decode(&csil_arr[1])?))
        }
        7 => {
            let csil_decode = csil_dec_membership_payload;
            Ok(EventPayload::Variant7(csil_decode(&csil_arr[1])?))
        }
        csil_other => Err(CsilCborError(format!(
            "csil cbor: unknown EventPayload variant {csil_other}"
        ))),
    }
}

/// Encode a StreamControl union as a tagged sum `[variant_index, value]`.
fn csil_enc_stream_control(csil_v: &StreamControl) -> CsilCborValue {
    match csil_v {
        StreamControl::Variant0(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(0),
            csil_enc_subscribe_request(csil_x),
        ]),
        StreamControl::Variant1(csil_x) => CsilCborValue::Array(vec![
            CsilCborValue::Uint(1),
            csil_enc_unsribe_marker(csil_x),
        ]),
    }
}

/// Decode a tagged sum `[variant_index, value]` into a StreamControl union.
fn csil_dec_stream_control(csil_v: &CsilCborValue) -> Result<StreamControl, CsilCborError> {
    let csil_arr = match csil_v {
        CsilCborValue::Array(csil_a) => csil_a,
        _ => {
            return Err(CsilCborError(
                "csil cbor: union expects a 2-element array".to_string(),
            ))
        }
    };
    if csil_arr.len() != 2 {
        return Err(CsilCborError(format!(
            "csil cbor: union array has {} elements, expected 2",
            csil_arr.len()
        )));
    }
    let csil_idx = cbor_as_u64(&csil_arr[0])?;
    match csil_idx {
        0 => {
            let csil_decode = csil_dec_subscribe_request;
            Ok(StreamControl::Variant0(csil_decode(&csil_arr[1])?))
        }
        1 => {
            let csil_decode = csil_dec_unsribe_marker;
            Ok(StreamControl::Variant1(csil_decode(&csil_arr[1])?))
        }
        csil_other => Err(CsilCborError(format!(
            "csil cbor: unknown StreamControl variant {csil_other}"
        ))),
    }
}

/// Encode a StreamMessage union as a tagged sum `[variant_index, value]`.
fn csil_enc_stream_message(csil_v: &StreamMessage) -> CsilCborValue {
    match csil_v {
        StreamMessage::Variant0(csil_x) => {
            CsilCborValue::Array(vec![CsilCborValue::Uint(0), csil_enc_event(csil_x)])
        }
        StreamMessage::Variant1(csil_x) => {
            CsilCborValue::Array(vec![CsilCborValue::Uint(1), csil_enc_stream_ack(csil_x)])
        }
    }
}

/// Decode a tagged sum `[variant_index, value]` into a StreamMessage union.
fn csil_dec_stream_message(csil_v: &CsilCborValue) -> Result<StreamMessage, CsilCborError> {
    let csil_arr = match csil_v {
        CsilCborValue::Array(csil_a) => csil_a,
        _ => {
            return Err(CsilCborError(
                "csil cbor: union expects a 2-element array".to_string(),
            ))
        }
    };
    if csil_arr.len() != 2 {
        return Err(CsilCborError(format!(
            "csil cbor: union array has {} elements, expected 2",
            csil_arr.len()
        )));
    }
    let csil_idx = cbor_as_u64(&csil_arr[0])?;
    match csil_idx {
        0 => {
            let csil_decode = csil_dec_event;
            Ok(StreamMessage::Variant0(csil_decode(&csil_arr[1])?))
        }
        1 => {
            let csil_decode = csil_dec_stream_ack;
            Ok(StreamMessage::Variant1(csil_decode(&csil_arr[1])?))
        }
        csil_other => Err(CsilCborError(format!(
            "csil cbor: unknown StreamMessage variant {csil_other}"
        ))),
    }
}
