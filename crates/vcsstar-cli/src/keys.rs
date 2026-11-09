//! `vcsstar keys`: developer SSH keys as LinkKeys application keys.
//!
//! The CLI makes Ed25519 keys, keeps them in OpenSSH files, enrolls this
//! machine as an application instance at the user's home domain
//! (`Account/enroll-application-instance`), and registers the public keys
//! with vcsstar (`Auth/register-key`). Renewal and revocation talk to the
//! home domain with the keys themselves; vcsstar only learns the result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use liblinkkeys::application_keys as ak;
use liblinkkeys::crypto::SigningAlgorithm;
use liblinkkeys::generated::types as lk;
use liblinkkeys::generated::{self as g};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

pub const APPLICATION_ID: &str = "vcsstar-client";
const KEY_LIFETIME_SECONDS: i64 = 365 * 86_400;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct KeyStore {
    #[serde(default)]
    pub enrollments: BTreeMap<String, Enrollment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Enrollment {
    /// `user_id@domain` of the developer.
    pub identity: String,
    pub subject_user_id: String,
    pub domain: String,
    pub application_id: String,
    pub instance_id: String,
    /// `host:port` of the home domain's TCP CSIL port.
    pub server: String,
    pub keys: Vec<LocalKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalKey {
    pub key_id: String,
    /// "auth" | "signing" | "quorum"
    pub role: String,
    pub path: PathBuf,
    /// LinkKeys fingerprint (hex SHA-256 of the raw public key).
    pub fingerprint: String,
    pub openssh_public: String,
    #[serde(default)]
    pub attested_at: Option<String>,
    #[serde(default)]
    pub attestation_expires_at: Option<String>,
    /// vcsstar's id for the registered key (auth and signing only).
    #[serde(default)]
    pub vcsstar_key_id: Option<String>,
}

pub fn store_path() -> PathBuf {
    if let Ok(p) = std::env::var("VCSSTAR_KEYS") {
        return PathBuf::from(p);
    }
    crate::profiles::path().with_file_name("keys.json")
}

pub fn load() -> Result<KeyStore> {
    match std::fs::read(store_path()) {
        Ok(b) => serde_json::from_slice(&b).context("parse keys.json"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(KeyStore::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save(store: &KeyStore) -> Result<()> {
    let p = store_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(store)?)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// Home-domain endpoint discovery: `_linkkeys_apis.<domain>` for the TCP
/// address and `_linkkeys.<domain>` for the TLS fingerprints.
pub struct HomeDomain {
    pub server: String,
    pub fingerprints: Vec<String>,
}

pub fn discover(domain: &str) -> Result<HomeDomain> {
    let apis = txt_records(&liblinkkeys::dns::linkkeys_apis_dns_name(domain))?;
    let mut server = None;
    for txt in &apis {
        if let Ok(rec) = liblinkkeys::dns::parse_linkkeys_apis_txt(txt) {
            if let Some(tcp) = rec.tcp {
                server = Some(if tcp.contains(':') {
                    tcp
                } else {
                    format!("{tcp}:4987")
                });
            }
        }
    }
    let server = server.unwrap_or_else(|| format!("linkkeys.{domain}:4987"));
    let anchors = txt_records(&liblinkkeys::dns::linkkeys_dns_name(domain))?;
    let mut fingerprints = Vec::new();
    for txt in &anchors {
        if let Ok(rec) = liblinkkeys::dns::parse_linkkeys_txt(txt) {
            fingerprints.extend(rec.fingerprints);
        }
    }
    if fingerprints.is_empty() {
        bail!(
            "no `_linkkeys.{domain}` TXT record with fingerprints; is {domain} a LinkKeys domain?"
        );
    }
    Ok(HomeDomain {
        server,
        fingerprints,
    })
}

fn txt_records(name: &str) -> Result<Vec<String>> {
    let name = format!("{}.", name.trim_end_matches('.'));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let resolver = hickory_resolver::TokioResolver::builder_tokio()
            .map_err(|e| anyhow!("resolver: {e}"))?
            .build()
            .map_err(|e| anyhow!("resolver: {e}"))?;
        let lookup = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            resolver.lookup(name.clone(), hickory_resolver::proto::rr::RecordType::TXT),
        )
        .await
        .map_err(|_| anyhow!("DNS lookup of {name} timed out"))?
        .map_err(|e| anyhow!("DNS lookup of {name}: {e}"))?;
        let mut out = Vec::new();
        for record in lookup.answers() {
            if let hickory_resolver::proto::rr::RData::TXT(txt) = &record.data {
                out.push(
                    txt.txt_data
                        .iter()
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .collect::<String>(),
                );
            }
        }
        Ok(out)
    })
}

pub struct Home<'a> {
    pub server: &'a str,
    pub fingerprints: &'a [String],
}

impl Home<'_> {
    pub fn call(
        &self,
        service: &str,
        op: &str,
        payload: Vec<u8>,
        api_key: Option<&str>,
    ) -> Result<Vec<u8>> {
        linkkeys_rpc_client::send_request(
            self.server,
            self.fingerprints.to_vec(),
            None,
            service,
            op,
            payload,
            api_key,
        )
        .map_err(|e| anyhow!("{service}/{op} at {}: {e}", self.server))
    }
}

/// Raw Ed25519 seed and public key from an OpenSSH private key file.
fn load_private(path: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let key = ssh_key::PrivateKey::read_openssh_file(path)
        .with_context(|| format!("read {}", path.display()))?;
    let kp = key
        .key_data()
        .ed25519()
        .ok_or_else(|| anyhow!("{} is not an Ed25519 key", path.display()))?;
    Ok((kp.private.to_bytes().to_vec(), kp.public.0.to_vec()))
}

fn write_private(path: &Path, key: &ssh_key::PrivateKey) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    if path.exists() {
        bail!("{} exists; remove it or pass --key-dir", path.display());
    }
    key.write_openssh_file(path, ssh_key::LineEnding::LF)
        .with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let pub_path = path.with_extension("pub");
    std::fs::write(
        &pub_path,
        format!(
            "{}\n",
            key.public_key()
                .to_openssh()
                .map_err(|e| anyhow!("encode public key: {e}"))?
        ),
    )?;
    Ok(())
}

fn now_window() -> (String, String) {
    let now = Utc::now();
    (
        (now - Duration::seconds(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        (now + Duration::seconds(120)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn decode_attestation(
    signed: &lk::SignedApplicationKeyAttestation,
) -> Result<lk::ApplicationKeyAttestation> {
    g::decode_application_key_attestation(&signed.attestation)
        .map_err(|e| anyhow!("attestation decode: {e}"))
}

pub struct EnrollOptions<'a> {
    pub profile: &'a str,
    pub identity: &'a str,
    pub api_key: &'a str,
    pub instance_id: Option<&'a str>,
    pub key_dir: PathBuf,
}

/// Make three keys, enroll the instance, return the enrollment (without the
/// vcsstar registration, which the caller does with the contract client).
pub fn enroll(opts: EnrollOptions<'_>) -> Result<Enrollment> {
    let (user_id, domain) = opts
        .identity
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("identity must be user_id@domain"))?;
    let home = discover(domain)?;
    let h = Home {
        server: &home.server,
        fingerprints: &home.fingerprints,
    };
    let me = g::decode_get_my_info_response(&h.call(
        "Account",
        "get-my-info",
        g::encode_empty_request(&lk::EmptyRequest {}),
        Some(opts.api_key),
    )?)
    .map_err(|e| anyhow!("get-my-info: {e}"))?;
    if !me.user.id.eq_ignore_ascii_case(user_id) {
        bail!(
            "the LinkKeys API key belongs to {}@{domain}, not to the vcsstar login {}",
            me.user.id,
            opts.identity
        );
    }
    let instance_id = match opts.instance_id {
        Some(i) => i.to_string(),
        None => {
            let host = hostname();
            format!("{host}-{}", &uuid::Uuid::now_v7().simple().to_string()[..8])
        }
    };
    let mut keys = Vec::new();
    let mut signed = Vec::new();
    for role in ["auth", "signing", "quorum"] {
        let key = ssh_key::PrivateKey::random(&mut rand::rngs::OsRng, ssh_key::Algorithm::Ed25519)
            .map_err(|e| anyhow!("keygen: {e}"))?;
        let mut key = key;
        key.set_comment(format!("vcsstar {role} {}@{domain}", user_id));
        let path = opts
            .key_dir
            .join(format!("vcsstar-{}-{role}", sanitize(opts.profile)));
        write_private(&path, &key)?;
        let (private, public) = load_private(&path)?;
        let key_id = format!(
            "{role}-{}",
            &uuid::Uuid::now_v7().simple().to_string()[..12]
        );
        let ch = g::decode_start_application_key_challenge_response(&h.call(
            "ApplicationKeys",
            "start-key-challenge",
            g::encode_start_application_key_challenge_request(
                &lk::StartApplicationKeyChallengeRequest {
                    subject_user_id: user_id.to_string(),
                    application_id: APPLICATION_ID.into(),
                    instance_id: instance_id.clone(),
                    purpose: ak::PURPOSE_ADD.into(),
                    key_usage: ak::KEY_USAGE_SIGN.into(),
                    algorithm: "ed25519".into(),
                    public_key: public.clone(),
                },
            ),
            None,
        )?)
        .map_err(|e| anyhow!("start-key-challenge: {e}"))?;
        let (requested_at, expires_at) = now_window();
        let addition = lk::ApplicationKeyAddition {
            subject_user_id: user_id.to_string(),
            subject_domain: domain.to_string(),
            application_id: APPLICATION_ID.into(),
            instance_id: instance_id.clone(),
            key_id: key_id.clone(),
            key_usage: ak::KEY_USAGE_SIGN.into(),
            algorithm: "ed25519".into(),
            public_key: public.clone(),
            fingerprint: liblinkkeys::crypto::fingerprint(&public),
            requested_key_lifetime_seconds: KEY_LIFETIME_SECONDS,
            challenge_id: ch.challenge_id,
            challenge: ch
                .challenge
                .ok_or_else(|| anyhow!("home domain sent no plaintext challenge"))?,
            requested_at,
            expires_at,
        };
        let signer = ak::ApplicationSigner {
            key_id: &key_id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &private,
        };
        signed.push(
            ak::sign_addition(&addition, &[], Some(&signer))
                .map_err(|e| anyhow!("sign addition: {e}"))?,
        );
        keys.push(LocalKey {
            key_id,
            role: role.into(),
            fingerprint: liblinkkeys::crypto::fingerprint(&public),
            openssh_public: key
                .public_key()
                .to_openssh()
                .map_err(|e| anyhow!("encode public key: {e}"))?,
            path,
            attested_at: None,
            attestation_expires_at: None,
            vcsstar_key_id: None,
        });
    }
    let resp = g::decode_enroll_application_instance_response(&h.call(
        "Account",
        "enroll-application-instance",
        g::encode_enroll_application_instance_request(&lk::EnrollApplicationInstanceRequest {
            application_id: APPLICATION_ID.into(),
            instance_id: instance_id.clone(),
            keys: signed,
        }),
        Some(opts.api_key),
    )?)
    .map_err(|e| anyhow!("enroll-application-instance: {e}"))?;
    for att in &resp.attestations {
        let a = decode_attestation(att)?;
        if let Some(k) = keys.iter_mut().find(|k| k.key_id == a.key_id) {
            k.attested_at = Some(a.attested_at.clone());
            k.attestation_expires_at = Some(a.attestation_expires_at.clone());
        }
    }
    Ok(Enrollment {
        identity: opts.identity.to_string(),
        subject_user_id: user_id.to_string(),
        domain: domain.to_string(),
        application_id: APPLICATION_ID.into(),
        instance_id,
        server: home.server,
        keys,
    })
}

/// Renew every attestation below half-life (or all with `force`). Returns
/// the keys whose attestation changed.
pub fn renew(e: &mut Enrollment, force: bool) -> Result<Vec<String>> {
    let home = discover(&e.domain)?;
    e.server = home.server.clone();
    let h = Home {
        server: &home.server,
        fingerprints: &home.fingerprints,
    };
    let now = Utc::now();
    let mut changed = Vec::new();
    for k in e.keys.iter_mut() {
        let due = match (&k.attested_at, &k.attestation_expires_at) {
            (Some(a), Some(x)) => ak::needs_new_attestation(a, x, now).unwrap_or(true),
            _ => true,
        };
        if !due && !force {
            continue;
        }
        let (private, public) = load_private(&k.path)?;
        let ch = g::decode_start_application_key_challenge_response(&h.call(
            "ApplicationKeys",
            "start-key-challenge",
            g::encode_start_application_key_challenge_request(
                &lk::StartApplicationKeyChallengeRequest {
                    subject_user_id: e.subject_user_id.clone(),
                    application_id: e.application_id.clone(),
                    instance_id: e.instance_id.clone(),
                    purpose: ak::PURPOSE_RENEW.into(),
                    key_usage: ak::KEY_USAGE_SIGN.into(),
                    algorithm: "ed25519".into(),
                    public_key: public.clone(),
                },
            ),
            None,
        )?)
        .map_err(|err| anyhow!("start-key-challenge: {err}"))?;
        let (requested_at, expires_at) = now_window();
        let renewal = lk::ApplicationKeyRenewal {
            subject_user_id: e.subject_user_id.clone(),
            subject_domain: e.domain.clone(),
            application_id: e.application_id.clone(),
            instance_id: e.instance_id.clone(),
            key_id: k.key_id.clone(),
            challenge_id: ch.challenge_id,
            challenge: ch
                .challenge
                .ok_or_else(|| anyhow!("home domain sent no plaintext challenge"))?,
            requested_at,
            expires_at,
        };
        let signer = ak::ApplicationSigner {
            key_id: &k.key_id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: &private,
        };
        let signed = ak::sign_renewal(&renewal, &[], Some(&signer))
            .map_err(|err| anyhow!("sign renewal: {err}"))?;
        let resp = g::decode_renew_application_key_attestation_response(&h.call(
            "ApplicationKeys",
            "renew-attestation",
            g::encode_renew_application_key_attestation_request(
                &lk::RenewApplicationKeyAttestationRequest { request: signed },
            ),
            None,
        )?)
        .map_err(|err| anyhow!("renew-attestation: {err}"))?;
        let a = decode_attestation(&resp.attestation)?;
        k.attested_at = Some(a.attested_at);
        k.attestation_expires_at = Some(a.attestation_expires_at);
        changed.push(k.key_id.clone());
    }
    Ok(changed)
}

/// Revoke one key with two sibling signatures at the home domain.
pub fn revoke(e: &Enrollment, target: &LocalKey) -> Result<String> {
    let home = discover(&e.domain)?;
    let h = Home {
        server: &home.server,
        fingerprints: &home.fingerprints,
    };
    let siblings: Vec<&LocalKey> = e
        .keys
        .iter()
        .filter(|k| k.key_id != target.key_id)
        .collect();
    if siblings.len() < 2 {
        bail!("revocation needs two other keys of this instance");
    }
    let loaded: Vec<(String, Vec<u8>)> = siblings
        .iter()
        .take(2)
        .map(|k| load_private(&k.path).map(|(p, _)| (k.key_id.clone(), p)))
        .collect::<Result<_>>()?;
    let signers: Vec<ak::ApplicationSigner<'_>> = loaded
        .iter()
        .map(|(id, p)| ak::ApplicationSigner {
            key_id: id,
            algorithm: SigningAlgorithm::Ed25519,
            private_key_bytes: p,
        })
        .collect();
    let revoked_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let rev = ak::sign_revocation(
        &ak::InstanceRef {
            subject_user_id: &e.subject_user_id,
            subject_domain: &e.domain,
            application_id: &e.application_id,
            instance_id: &e.instance_id,
        },
        &target.key_id,
        &target.fingerprint,
        &revoked_at,
        &signers,
    )
    .map_err(|err| anyhow!("sign revocation: {err}"))?;
    let resp = g::decode_revoke_application_key_response(&h.call(
        "ApplicationKeys",
        "revoke-key",
        g::encode_revoke_application_key_request(&lk::RevokeApplicationKeyRequest {
            revocation: rev,
        }),
        None,
    )?)
    .map_err(|err| anyhow!("revoke-key: {err}"))?;
    Ok(resp.revoked_at)
}

pub fn status_json(e: &Enrollment) -> Json {
    let now = Utc::now();
    json!({
        "identity": e.identity,
        "application_id": e.application_id,
        "instance_id": e.instance_id,
        "home_server": e.server,
        "keys": e.keys.iter().map(|k| {
            let expires = k.attestation_expires_at.as_deref().and_then(|s| DateTime::parse_from_rfc3339(s).ok());
            let state = match expires {
                Some(x) if x.with_timezone(&Utc) <= now => "attestation expired",
                Some(_) => "attested",
                None => "not attested",
            };
            json!({
                "role": k.role, "key_id": k.key_id, "path": k.path,
                "fingerprint": k.fingerprint,
                "attested_at": k.attested_at, "attestation_expires_at": k.attestation_expires_at,
                "state": state, "vcsstar_key_id": k.vcsstar_key_id,
            })
        }).collect::<Vec<_>>(),
    })
}

/// Write the git signing configuration for the enrollment.
pub fn git_config(
    e: &Enrollment,
    global: bool,
    allowed_signers: Option<&Path>,
) -> Result<Vec<String>> {
    let signing = e
        .keys
        .iter()
        .find(|k| k.role == "signing")
        .ok_or_else(|| anyhow!("no signing key in this enrollment"))?;
    let mut settings = vec![
        ("gpg.format".to_string(), "ssh".to_string()),
        (
            "user.signingkey".to_string(),
            signing.path.display().to_string(),
        ),
        ("commit.gpgsign".to_string(), "true".to_string()),
        ("tag.gpgsign".to_string(), "true".to_string()),
    ];
    if let Some(p) = allowed_signers {
        settings.push((
            "gpg.ssh.allowedSignersFile".to_string(),
            p.display().to_string(),
        ));
    }
    let mut applied = Vec::new();
    for (k, v) in &settings {
        let mut c = std::process::Command::new("git");
        c.arg("config");
        if global {
            c.arg("--global");
        }
        c.arg(k).arg(v);
        let st = c.status().context("run git config")?;
        if !st.success() {
            bail!("git config {k} failed");
        }
        applied.push(format!("{k}={v}"));
    }
    Ok(applied)
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "host".into())
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
