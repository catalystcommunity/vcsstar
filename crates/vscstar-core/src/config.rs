//! Environment-first configuration. Prefix `VSCSTAR__` (double underscore, so
//! Kubernetes Service links such as `VSCSTAR_PORT` never match). Secrets use
//! `SECRET_VSCSTAR__` and are never logged.

use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub db_pool_size: u32,
    pub listen_http: String,
    pub listen_rpc: String,
    /// Public origin, e.g. `https://vscstar.catalystsquad.com`.
    pub public_url: String,
    /// Hostname domain of this instance, e.g. `catalystsquad.com`.
    pub instance_domain: String,
    /// LinkKeys account of this instance, e.g. `vscstar@catalystsquad.com`.
    pub instance_identity: String,
    pub linkkeys: LinkKeysConfig,
    pub corndogs_addr: Option<String>,
    pub corndogs_tls: bool,
    pub object_store: Option<ObjectStoreConfig>,
    /// `user_id@domain` identities that may create organizations and act as
    /// admin everywhere (`VSCSTAR__INSTANCE_ADMINS`, comma separated).
    pub instance_admins: Vec<String>,
    /// When true, any authenticated identity may create an organization.
    pub open_org_creation: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LinkKeysConfig {
    /// `host:port` of the domain linkkeys server's TCP CSIL port (in-cluster).
    pub rp_addr: Option<String>,
    /// Raw Ed25519 SPKI SHA-256 fingerprints, comma separated.
    pub rp_fingerprints: Vec<String>,
    /// The RP server's own domain; assertions are audience-checked against it.
    pub rp_domain: Option<String>,
    /// API key for the `Rp` service (`SECRET_VSCSTAR__LINKKEYS_API_KEY`).
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ObjectStoreConfig {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
}

fn var(name: &str) -> Option<String> {
    env::var(format!("VSCSTAR__{name}"))
        .ok()
        .filter(|v| !v.is_empty())
}

fn secret(name: &str) -> Option<String> {
    env::var(format!("SECRET_VSCSTAR__{name}"))
        .ok()
        .filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = var("DATABASE_URL")
            .or_else(|| secret("DATABASE_URL"))
            .or_else(|| env::var("DATABASE_URL").ok())
            .ok_or("VSCSTAR__DATABASE_URL is required")?;
        let instance_domain = var("INSTANCE_DOMAIN").unwrap_or_else(|| "localhost".into());
        let object_store = match (var("OBJECT_STORE_ENDPOINT"), var("OBJECT_STORE_BUCKET")) {
            (Some(endpoint), Some(bucket)) => Some(ObjectStoreConfig {
                endpoint,
                bucket,
                access_key: var("OBJECT_STORE_ACCESS_KEY")
                    .or_else(|| secret("OBJECT_STORE_ACCESS_KEY"))
                    .unwrap_or_default(),
                secret_key: var("OBJECT_STORE_SECRET_KEY")
                    .or_else(|| secret("OBJECT_STORE_SECRET_KEY"))
                    .unwrap_or_default(),
                region: var("OBJECT_STORE_REGION").unwrap_or_else(|| "us-east-1".into()),
            }),
            _ => None,
        };
        Ok(Self {
            database_url,
            db_pool_size: var("DB_POOL_SIZE")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
            listen_http: var("LISTEN_HTTP").unwrap_or_else(|| "127.0.0.1:8080".into()),
            listen_rpc: var("LISTEN_RPC").unwrap_or_else(|| "127.0.0.1:4988".into()),
            public_url: var("PUBLIC_URL").unwrap_or_else(|| "http://127.0.0.1:8080".into()),
            instance_identity: var("INSTANCE_IDENTITY")
                .unwrap_or_else(|| format!("vscstar@{instance_domain}")),
            instance_domain,
            linkkeys: LinkKeysConfig {
                rp_addr: var("LINKKEYS_RP_ADDR"),
                rp_fingerprints: var("LINKKEYS_RP_FINGERPRINTS")
                    .map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                rp_domain: var("LINKKEYS_RP_DOMAIN"),
                api_key: secret("LINKKEYS_API_KEY"),
            },
            corndogs_addr: var("CORNDOGS_ADDR"),
            corndogs_tls: var("CORNDOGS_TLS").map(|v| v == "true").unwrap_or(true),
            object_store,
            instance_admins: var("INSTANCE_ADMINS")
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            open_org_creation: var("OPEN_ORG_CREATION")
                .map(|v| v == "true")
                .unwrap_or(false),
        })
    }
}
