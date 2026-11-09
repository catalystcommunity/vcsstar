//! The instance's own LinkKeys application keys: application `vcsstar`,
//! one instance id per deployment, three Ed25519 signing keys enrolled at
//! the instance account's home domain with the account API key the RP
//! client already holds. Seeds are sealed in Postgres with the credential
//! key. They sign merge commits (SSH signature format) and act-as scope
//! sets.

use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use liblinkkeys::application_keys as ak;
use liblinkkeys::crypto::SigningAlgorithm;
use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{self as g};
use vcsstar_db::schema::instance_keys;
use vcsstar_db::DbConn;
use vcsstar_vcs::Signer;

use crate::mirror::cred;
use crate::{App, Code, Error};

pub const APPLICATION_ID: &str = "vcsstar";
const KEY_LIFETIME_SECONDS: i64 = 365 * 86_400;

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = instance_keys)]
pub struct KeyRow {
    pub key_id: String,
    pub instance_id: String,
    pub role: String,
    pub public_key: Vec<u8>,
    pub private_sealed: Vec<u8>,
    pub attested_at: Option<DateTime<Utc>>,
    pub attestation_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// A usable signing key: seed in memory for the duration of one operation.
pub struct Unsealed {
    pub key_id: String,
    pub public: Vec<u8>,
    pub seed: Vec<u8>,
}

/// An OpenSSH private key file for git, removed on drop.
pub struct SshSigner {
    pub signer: Signer,
    _file: tempfile::NamedTempFile,
}

/// `user_id@domain` of the instance account, from configuration.
pub fn account(app: &App) -> Result<(String, String), Error> {
    let id = &app.config.instance_identity;
    let (user, domain) = id.rsplit_once('@').ok_or_else(|| {
        Error::new(
            Code::PreconditionFailed,
            "VCSSTAR__INSTANCE_IDENTITY must be user_id@domain",
        )
    })?;
    Ok((user.to_string(), domain.to_string()))
}

pub fn instance_id(app: &App) -> String {
    app.config
        .public_url
        .split("://")
        .nth(1)
        .unwrap_or(&app.config.public_url)
        .trim_end_matches('/')
        .replace([':', '/'], "-")
}

pub fn load(conn: &mut PgConnection) -> Result<Vec<KeyRow>, Error> {
    Ok(instance_keys::table
        .filter(instance_keys::revoked_at.is_null())
        .order(instance_keys::created_at.asc())
        .select(KeyRow::as_select())
        .load(conn)?)
}

pub fn unseal(app: &App, row: &KeyRow) -> Result<Unsealed, Error> {
    let seed_hex = cred::open(app, &row.private_sealed)?;
    let seed = hex::decode(seed_hex.trim())
        .map_err(|_| Error::internal("instance key seed is not hex"))?;
    Ok(Unsealed {
        key_id: row.key_id.clone(),
        public: row.public_key.clone(),
        seed,
    })
}

/// Home-domain TCP caller for the instance account: the RP connection.
fn home(app: &App) -> Result<crate::linkkeys::Rp<'_>, Error> {
    app.rp()
}

fn now_window() -> (String, String) {
    let now = Utc::now();
    (
        (now - Duration::seconds(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        (now + Duration::seconds(120)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Enroll three signing keys when none exist. Needs the credential key (to
/// seal seeds) and the RP connection (the account API key). Safe to call at
/// every start; a no-op when keys are present.
pub fn ensure_enrolled(app: &App, conn: &mut DbConn) -> Result<usize, Error> {
    if !load(conn)?.is_empty() {
        return Ok(0);
    }
    if app.config.credential_key.is_none() {
        return Err(Error::new(
            Code::PreconditionFailed,
            "instance keys need SECRET_VCSSTAR__CREDENTIAL_KEY",
        ));
    }
    let (user_id, domain) = account(app)?;
    let rp = home(app)?;
    let instance = instance_id(app);
    let mut rows = Vec::new();
    let mut signed = Vec::new();
    for role in ["signing-a", "signing-b", "signing-c"] {
        let (verifying, signing) = liblinkkeys::crypto::generate_ed25519_keypair();
        let public = verifying.to_bytes().to_vec();
        let seed = signing.to_bytes().to_vec();
        let key_id = format!(
            "{role}-{}",
            &uuid::Uuid::now_v7().simple().to_string()[..10]
        );
        let ch = g::decode_start_application_key_challenge_response(&rp.caller.call_service(
            "ApplicationKeys",
            "start-key-challenge",
            g::encode_start_application_key_challenge_request(
                &lk::StartApplicationKeyChallengeRequest {
                    subject_user_id: user_id.clone(),
                    application_id: APPLICATION_ID.into(),
                    instance_id: instance.clone(),
                    purpose: ak::PURPOSE_ADD.into(),
                    key_usage: ak::KEY_USAGE_SIGN.into(),
                    algorithm: "ed25519".into(),
                    public_key: public.clone(),
                },
            ),
            false,
        )?)
        .map_err(|e| Error::new(Code::Unavailable, format!("start-key-challenge: {e}")))?;
        let (requested_at, expires_at) = now_window();
        let addition = lk::ApplicationKeyAddition {
            subject_user_id: user_id.clone(),
            subject_domain: domain.clone(),
            application_id: APPLICATION_ID.into(),
            instance_id: instance.clone(),
            key_id: key_id.clone(),
            key_usage: ak::KEY_USAGE_SIGN.into(),
            algorithm: "ed25519".into(),
            public_key: public.clone(),
            fingerprint: liblinkkeys::crypto::fingerprint(&public),
            requested_key_lifetime_seconds: KEY_LIFETIME_SECONDS,
            challenge_id: ch.challenge_id,
            challenge: ch
                .challenge
                .ok_or_else(|| Error::new(Code::Unavailable, "no plaintext challenge"))?,
            requested_at,
            expires_at,
        };
        let signer = ak::ApplicationSigner {
            key_id: &key_id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &seed,
        };
        signed.push(
            ak::sign_addition(&addition, &[], Some(&signer))
                .map_err(|e| Error::internal(e.to_string()))?,
        );
        rows.push((key_id, role.to_string(), public, seed));
    }
    let resp = g::decode_enroll_application_instance_response(&rp.caller.call_service(
        "Account",
        "enroll-application-instance",
        g::encode_enroll_application_instance_request(&lk::EnrollApplicationInstanceRequest {
            application_id: APPLICATION_ID.into(),
            instance_id: instance.clone(),
            keys: signed,
        }),
        true,
    )?)
    .map_err(|e| {
        Error::new(
            Code::Unavailable,
            format!("enroll-application-instance: {e}"),
        )
    })?;
    for (key_id, role, public, seed) in rows {
        let att = resp
            .attestations
            .iter()
            .filter_map(|a| g::decode_application_key_attestation(&a.attestation).ok())
            .find(|a| a.key_id == key_id);
        let sealed = cred::seal(app, hex::encode(&seed).as_bytes())?;
        diesel::insert_into(instance_keys::table)
            .values((
                instance_keys::key_id.eq(&key_id),
                instance_keys::instance_id.eq(&instance),
                instance_keys::role.eq(&role),
                instance_keys::public_key.eq(&public),
                instance_keys::private_sealed.eq(&sealed),
                instance_keys::attested_at
                    .eq(att.as_ref().and_then(|a| parse_time(&a.attested_at))),
                instance_keys::attestation_expires_at.eq(att
                    .as_ref()
                    .and_then(|a| parse_time(&a.attestation_expires_at))),
            ))
            .execute(conn)?;
    }
    tracing::info!(instance = %instance, "instance application keys enrolled");
    Ok(3)
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Renew attestations below half-life. Call every few hours.
pub fn renew(app: &App, conn: &mut DbConn) -> Result<usize, Error> {
    let rows = load(conn)?;
    if rows.is_empty() {
        return Ok(0);
    }
    let (user_id, domain) = account(app)?;
    let rp = home(app)?;
    let now = Utc::now();
    let mut renewed = 0;
    for row in &rows {
        let due = match (row.attested_at, row.attestation_expires_at) {
            (Some(a), Some(x)) => {
                ak::needs_new_attestation(&a.to_rfc3339(), &x.to_rfc3339(), now).unwrap_or(true)
            }
            _ => true,
        };
        if !due {
            continue;
        }
        let key = unseal(app, row)?;
        let ch = g::decode_start_application_key_challenge_response(&rp.caller.call_service(
            "ApplicationKeys",
            "start-key-challenge",
            g::encode_start_application_key_challenge_request(
                &lk::StartApplicationKeyChallengeRequest {
                    subject_user_id: user_id.clone(),
                    application_id: APPLICATION_ID.into(),
                    instance_id: row.instance_id.clone(),
                    purpose: ak::PURPOSE_RENEW.into(),
                    key_usage: ak::KEY_USAGE_SIGN.into(),
                    algorithm: "ed25519".into(),
                    public_key: key.public.clone(),
                },
            ),
            false,
        )?)
        .map_err(|e| Error::new(Code::Unavailable, format!("start-key-challenge: {e}")))?;
        let (requested_at, expires_at) = now_window();
        let renewal = lk::ApplicationKeyRenewal {
            subject_user_id: user_id.clone(),
            subject_domain: domain.clone(),
            application_id: APPLICATION_ID.into(),
            instance_id: row.instance_id.clone(),
            key_id: row.key_id.clone(),
            challenge_id: ch.challenge_id,
            challenge: ch
                .challenge
                .ok_or_else(|| Error::new(Code::Unavailable, "no plaintext challenge"))?,
            requested_at,
            expires_at,
        };
        let signer = ak::ApplicationSigner {
            key_id: &row.key_id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &key.seed,
        };
        let signed = ak::sign_renewal(&renewal, &[], Some(&signer))
            .map_err(|e| Error::internal(e.to_string()))?;
        let resp = g::decode_renew_application_key_attestation_response(&rp.caller.call_service(
            "ApplicationKeys",
            "renew-attestation",
            g::encode_renew_application_key_attestation_request(
                &lk::RenewApplicationKeyAttestationRequest { request: signed },
            ),
            false,
        )?)
        .map_err(|e| Error::new(Code::Unavailable, format!("renew-attestation: {e}")))?;
        if let Ok(a) = g::decode_application_key_attestation(&resp.attestation.attestation) {
            diesel::update(instance_keys::table.find(&row.key_id))
                .set((
                    instance_keys::attested_at.eq(parse_time(&a.attested_at)),
                    instance_keys::attestation_expires_at.eq(parse_time(&a.attestation_expires_at)),
                ))
                .execute(conn)?;
        }
        renewed += 1;
    }
    Ok(renewed)
}

/// The signer for server-made commits: the first attested key as an
/// OpenSSH private key file. `None` when the instance has no keys yet, so
/// merges stay unsigned rather than failing.
pub fn signer(app: &App, conn: &mut DbConn) -> Result<Option<SshSigner>, Error> {
    let rows = load(conn)?;
    let Some(row) = rows
        .iter()
        .find(|r| r.attestation_expires_at.is_some_and(|x| x > Utc::now()))
        .or(rows.first())
    else {
        return Ok(None);
    };
    let key = unseal(app, row)?;
    let seed: [u8; 32] = key
        .seed
        .as_slice()
        .try_into()
        .map_err(|_| Error::internal("instance key seed length"))?;
    let kp = ssh_key::private::Ed25519Keypair::from_seed(&seed);
    let mut private = ssh_key::PrivateKey::from(kp);
    private.set_comment(format!("vcsstar {} {}", row.instance_id, row.key_id));
    let file = tempfile::Builder::new()
        .prefix("vcsstar-sign-")
        .tempfile()
        .map_err(Error::internal)?;
    private
        .write_openssh_file(file.path(), ssh_key::LineEnding::LF)
        .map_err(|e| Error::internal(format!("write signing key: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600));
    }
    Ok(Some(SshSigner {
        signer: Signer {
            private_key_path: file.path().to_path_buf(),
        },
        _file: file,
    }))
}

/// The instance's public signing keys in OpenSSH form (for allowed-signers
/// files and the UI).
pub fn public_openssh(conn: &mut PgConnection) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for row in load(conn)? {
        let bytes: [u8; 32] = match row.public_key.as_slice().try_into() {
            Ok(b) => b,
            Err(_) => continue,
        };
        let pk = ssh_key::public::Ed25519PublicKey(bytes);
        let key = ssh_key::PublicKey::from(pk);
        if let Ok(s) = key.to_openssh() {
            out.push(s);
        }
    }
    Ok(out)
}

/// Application signers for act-as scope sets: every unrevoked key.
pub fn application_signers(app: &App, conn: &mut PgConnection) -> Result<Vec<Unsealed>, Error> {
    let mut out = Vec::new();
    for row in load(conn)? {
        out.push(unseal(app, &row)?);
    }
    Ok(out)
}
