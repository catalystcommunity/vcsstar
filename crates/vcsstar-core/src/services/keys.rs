//! SSH keys: registration, listing, revocation, and the allowed-signers set
//! used for commit signature verification. Attested keys are LinkKeys
//! application keys; the instance verifies them through its RP.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use ssh_key::{HashAlg, PublicKey};
use vcsstar_db::models::Identity;
use vcsstar_db::schema::{identities, ssh_keys};
use vcsstar_db::DbConn;
use vcsstar_types::{
    Empty, KeyRef, ListKeysResponse, Ok as WireOk, RegisterKeyRequest, Signature, SshKeyInfo,
};
use vcsstar_vcs::{Env, RevisionInfo, SigStatus, VcsBackend};

use crate::principal::{identity_to_wire, ReqCtx};
use crate::{from_ms, to_ms, App, Code, Error};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = ssh_keys)]
pub struct KeyRow {
    pub id: uuid::Uuid,
    pub identity_id: uuid::Uuid,
    pub usage: String,
    pub algo: String,
    pub public_key: Vec<u8>,
    pub fingerprint: String,
    pub attested: bool,
    pub application_id: Option<String>,
    pub instance_id: Option<String>,
    pub key_id: Option<String>,
    pub attested_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl KeyRow {
    pub fn status(&self) -> &'static str {
        if self.revoked_at.is_some() {
            "revoked"
        } else if self.attested {
            "attested"
        } else {
            "unattested"
        }
    }

    pub fn openssh(&self) -> String {
        String::from_utf8_lossy(&self.public_key).trim().to_string()
    }

    /// Was this key an acceptable signer at `at`?
    pub fn valid_at(&self, at: DateTime<Utc>) -> bool {
        self.created_at <= at + chrono::Duration::minutes(5)
            && self.revoked_at.is_none_or(|r| r > at)
    }

    /// Was the attestation current at `at`? Expiry later is a missing proof,
    /// not a revocation (plan/01 time rules).
    pub fn attested_at(&self, at: DateTime<Utc>) -> bool {
        self.attested && self.valid_at(at) && self.attested_until.is_none_or(|u| u >= at)
    }
}

fn to_wire(k: &KeyRow) -> SshKeyInfo {
    SshKeyInfo {
        key_id: k.id.to_string(),
        public_key: k.openssh(),
        usage: k.usage.clone(),
        status: k.status().to_string(),
        application_id: k.application_id.clone(),
        instance_id: k.instance_id.clone(),
        attested_until: k.attested_until.map(to_ms),
        created_at: to_ms(k.created_at),
    }
}

/// Parse an OpenSSH public key line; returns (algorithm, fingerprint, canonical line).
pub fn parse_public_key(line: &str) -> Result<(String, String, String), Error> {
    let key = PublicKey::from_openssh(line.trim())
        .map_err(|e| Error::invalid(format!("public key is not an OpenSSH key line: {e}")))?;
    let algo = key.algorithm().to_string();
    if !matches!(
        algo.as_str(),
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
    ) {
        return Err(Error::invalid(format!("unsupported key algorithm {algo}")));
    }
    let fp = key.fingerprint(HashAlg::Sha256).to_string();
    let canonical = key
        .to_openssh()
        .map_err(|e| Error::internal(format!("re-encode key: {e}")))?;
    Ok((algo, fp, canonical))
}

pub fn register(app: &App, ctx: &ReqCtx, req: RegisterKeyRequest) -> Result<SshKeyInfo, Error> {
    let p = ctx.require()?;
    p.require_scope("keys:write")?;
    if !["auth", "signing"].contains(&req.usage.as_str()) {
        return Err(Error::invalid("usage must be auth or signing"));
    }
    let (algo, fingerprint, canonical) = parse_public_key(&req.public_key)?;
    let coords = match (&req.application_id, &req.instance_id, &req.lk_key_id) {
        (None, None, None) => None,
        (Some(a), Some(i), Some(k)) => Some((a.clone(), i.clone(), k.clone())),
        _ => {
            return Err(Error::invalid(
                "application_id, instance_id, and lk_key_id go together",
            ))
        }
    };
    let mut attested_until = None;
    if let Some((application_id, instance_id, key_id)) = &coords {
        if algo != "ssh-ed25519" {
            return Err(Error::invalid("attested keys must be Ed25519"));
        }
        let resolved = crate::linkkeys::resolve_application_key(
            app,
            &p.identity.lk_user_id.to_string(),
            &p.identity.lk_domain,
            application_id,
            instance_id,
            key_id,
        )?;
        let pk = PublicKey::from_openssh(&canonical).map_err(|e| Error::internal(e.to_string()))?;
        let raw = match pk.key_data().ed25519() {
            Some(k) => k.0.to_vec(),
            None => return Err(Error::invalid("not an Ed25519 key")),
        };
        if resolved.public_key != raw {
            return Err(Error::new(
                Code::PreconditionFailed,
                "the attested key does not match the submitted public key",
            ));
        }
        attested_until = Some(resolved.attestation_expires_at);
    }
    let mut conn = app.conn()?;
    let existing: Option<KeyRow> = ssh_keys::table
        .filter(ssh_keys::fingerprint.eq(&fingerprint))
        .select(KeyRow::as_select())
        .first(&mut conn)
        .optional()?;
    if let Some(e) = existing {
        if e.identity_id != p.identity.id {
            return Err(Error::new(
                Code::AlreadyExists,
                "this key is registered to another identity",
            ));
        }
        if e.revoked_at.is_some() {
            return Err(Error::new(
                Code::PreconditionFailed,
                "this key was revoked; make a new key",
            ));
        }
        // Re-registration refreshes the attestation window.
        let updated: KeyRow = diesel::update(ssh_keys::table.find(e.id))
            .set((
                ssh_keys::usage.eq(&req.usage),
                ssh_keys::attested.eq(coords.is_some()),
                ssh_keys::application_id.eq(coords.as_ref().map(|c| c.0.clone())),
                ssh_keys::instance_id.eq(coords.as_ref().map(|c| c.1.clone())),
                ssh_keys::key_id.eq(coords.as_ref().map(|c| c.2.clone())),
                ssh_keys::attested_until.eq(attested_until),
            ))
            .returning(KeyRow::as_returning())
            .get_result(&mut conn)?;
        return Ok(to_wire(&updated));
    }
    let row: KeyRow = diesel::insert_into(ssh_keys::table)
        .values((
            ssh_keys::id.eq(vcsstar_db::new_id()),
            ssh_keys::identity_id.eq(p.identity.id),
            ssh_keys::usage.eq(&req.usage),
            ssh_keys::algo.eq(&algo),
            ssh_keys::public_key.eq(canonical.as_bytes()),
            ssh_keys::fingerprint.eq(&fingerprint),
            ssh_keys::attested.eq(coords.is_some()),
            ssh_keys::application_id.eq(coords.as_ref().map(|c| c.0.clone())),
            ssh_keys::instance_id.eq(coords.as_ref().map(|c| c.1.clone())),
            ssh_keys::key_id.eq(coords.as_ref().map(|c| c.2.clone())),
            ssh_keys::attested_until.eq(attested_until),
        ))
        .returning(KeyRow::as_returning())
        .get_result(&mut conn)?;
    Ok(to_wire(&row))
}

pub fn list(app: &App, ctx: &ReqCtx, _req: Empty) -> Result<ListKeysResponse, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    let rows: Vec<KeyRow> = ssh_keys::table
        .filter(ssh_keys::identity_id.eq(p.identity.id))
        .order(ssh_keys::created_at.asc())
        .select(KeyRow::as_select())
        .load(&mut conn)?;
    Ok(ListKeysResponse {
        keys: rows.iter().map(to_wire).collect(),
    })
}

pub fn revoke(app: &App, ctx: &ReqCtx, req: KeyRef) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    p.require_scope("keys:write")?;
    let id = uuid::Uuid::parse_str(&req.key_id).map_err(|_| Error::invalid("key_id"))?;
    let mut conn = app.conn()?;
    let n = diesel::update(
        ssh_keys::table
            .filter(ssh_keys::id.eq(id))
            .filter(ssh_keys::identity_id.eq(p.identity.id))
            .filter(ssh_keys::revoked_at.is_null()),
    )
    .set(ssh_keys::revoked_at.eq(Utc::now()))
    .execute(&mut conn)?;
    if n == 0 {
        return Err(Error::not_found("key"));
    }
    Ok(WireOk { ok: true })
}

/// Resolve an SSH auth key presented to the SSH server.
pub fn identity_for_auth_key(
    conn: &mut DbConn,
    fingerprint: &str,
) -> Result<Option<(Identity, KeyRow)>, Error> {
    let row: Option<(KeyRow, Identity)> = ssh_keys::table
        .inner_join(identities::table)
        .filter(ssh_keys::fingerprint.eq(fingerprint))
        .filter(ssh_keys::revoked_at.is_null())
        .select((KeyRow::as_select(), Identity::as_select()))
        .first(conn)
        .optional()?;
    let Some((key, identity)) = row else {
        return Ok(None);
    };
    if identity.disabled_at.is_some() {
        return Ok(None);
    }
    // Attested keys need a current attestation for live operations.
    if key.attested && !key.attested_at(Utc::now()) {
        return Ok(None);
    }
    Ok(Some((identity, key)))
}

/// The allowed-signers set: every registered signing key, principal =
/// `user_id@domain`. Written to a temp file for `git verify-commit`.
pub struct AllowedSigners {
    pub path: PathBuf,
    _file: tempfile::NamedTempFile,
    keys: Vec<(KeyRow, Identity)>,
    /// (SSH fingerprint, key id) of the instance's own signing keys.
    instance: Vec<(String, String)>,
}

impl AllowedSigners {
    pub fn build(conn: &mut DbConn) -> Result<Self, Error> {
        let keys: Vec<(KeyRow, Identity)> = ssh_keys::table
            .inner_join(identities::table)
            .filter(ssh_keys::usage.eq("signing"))
            .select((KeyRow::as_select(), Identity::as_select()))
            .load(conn)?;
        let mut body = String::new();
        for (k, i) in &keys {
            let line = k.openssh();
            let mut parts = line.split_whitespace();
            if let (Some(t), Some(b)) = (parts.next(), parts.next()) {
                body.push_str(&format!(
                    "{}@{} namespaces=\"git\" {t} {b}\n",
                    i.lk_user_id, i.lk_domain
                ));
            }
        }
        // The instance's own keys sign merge commits; principal is the
        // instance account.
        let mut instance: Vec<(String, String)> = Vec::new();
        for k in crate::instance_keys::load(conn)? {
            if let Ok(b) = <[u8; 32]>::try_from(k.public_key.as_slice()) {
                let pk = ssh_key::PublicKey::from(ssh_key::public::Ed25519PublicKey(b));
                if let Ok(line) = pk.to_openssh() {
                    let mut parts = line.split_whitespace();
                    if let (Some(t), Some(b64)) = (parts.next(), parts.next()) {
                        body.push_str(&format!("vcsstar-instance namespaces=\"git\" {t} {b64}\n"));
                        instance.push((
                            pk.fingerprint(HashAlg::Sha256).to_string(),
                            k.key_id.clone(),
                        ));
                    }
                }
            }
        }
        let mut file = tempfile::NamedTempFile::new().map_err(Error::internal)?;
        std::io::Write::write_all(&mut file, body.as_bytes()).map_err(Error::internal)?;
        Ok(Self {
            path: file.path().to_path_buf(),
            _file: file,
            keys,
            instance,
        })
    }

    /// Classify a revision's signature with the time rules of plan/01.
    pub fn classify(
        &self,
        _conn: &mut DbConn,
        backend: &dyn VcsBackend,
        dir: &Path,
        rev: &RevisionInfo,
        env: &Env,
    ) -> Result<Signature, Error> {
        if rev.signature.is_none() {
            return Ok(Signature {
                status: "missing".into(),
                signer: None,
                key_fingerprint: None,
            });
        }
        let status = backend.verify_signature(dir, &rev.id, &self.path, env)?;
        Ok(match status {
            SigStatus::Good {
                principal,
                fingerprint,
            } => {
                let at = from_ms(rev.committed_at_ms);
                let found = self.keys.iter().find(|(k, _)| k.fingerprint == fingerprint);
                let (status, signer) = match found {
                    Some((k, i)) if k.attested_at(at) => ("verified", Some(identity_to_wire(i))),
                    Some((k, i)) if k.valid_at(at) => {
                        ("valid_unattested", Some(identity_to_wire(i)))
                    }
                    Some((_, i)) => ("unverified", Some(identity_to_wire(i))),
                    None if self.instance.iter().any(|(fp, _)| *fp == fingerprint) => {
                        ("verified", None)
                    }
                    None => ("unverified", None),
                };
                let _ = principal;
                Signature {
                    status: status.into(),
                    signer,
                    key_fingerprint: Some(fingerprint),
                }
            }
            SigStatus::Bad { .. } => Signature {
                status: "unverified".into(),
                signer: None,
                key_fingerprint: None,
            },
            SigStatus::Missing => Signature {
                status: "missing".into(),
                signer: None,
                key_fingerprint: None,
            },
        })
    }
}
