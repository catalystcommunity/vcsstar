//! Environment-first configuration. Prefix `VCSSTAR__` (double underscore, so
//! Kubernetes Service links such as `VCSSTAR_PORT` never match). Secrets use
//! `SECRET_VCSSTAR__` and are never logged.

use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub db_pool_size: u32,
    pub listen_http: String,
    pub listen_rpc: String,
    /// Public origin, e.g. `https://vcsstar.catalystsquad.com`.
    pub public_url: String,
    /// Hostname domain of this instance, e.g. `catalystsquad.com`.
    pub instance_domain: String,
    /// LinkKeys account of this instance, e.g. `vcsstar@catalystsquad.com`.
    pub instance_identity: String,
    pub linkkeys: LinkKeysConfig,
    pub corndogs_addr: Option<String>,
    pub corndogs_tls: bool,
    pub object_store: Option<ObjectStoreConfig>,
    /// `user_id@domain` identities that may create organizations and act as
    /// admin everywhere (`VCSSTAR__INSTANCE_ADMINS`, comma separated).
    pub instance_admins: Vec<String>,
    /// When true, any authenticated identity may create an organization.
    pub open_org_creation: bool,
    /// Node-local cache of repository directories (`VCSSTAR__REPO_CACHE_DIR`).
    /// Disposable: refs live in Postgres, history in the object store.
    pub repo_cache_dir: std::path::PathBuf,
    /// Unix socket the pre-receive hook calls back on.
    pub hook_socket: std::path::PathBuf,
    /// Command the hook scripts exec (`<vcsstard> hook`). Defaults to the
    /// running executable.
    pub hook_command: String,
    /// SSH listener for the `git-ssh` role (`VCSSTAR__LISTEN_SSH`).
    pub listen_ssh: String,
    /// Path of the SSH host key (OpenSSH private key format). Generated when
    /// absent, which is fine for development and wrong for production: mount
    /// a Secret there.
    pub ssh_host_key_path: std::path::PathBuf,
    /// `host[:port]` clients use for SSH clone URLs (`VCSSTAR__SSH_PUBLIC_HOST`).
    /// Unset means SSH URLs are not advertised.
    pub ssh_public_host: Option<String>,
    /// 32-byte key (base64) that seals stored mirror credentials
    /// (`SECRET_VCSSTAR__CREDENTIAL_KEY`). Unset: only `env:` refs work.
    pub credential_key: Option<[u8; 32]>,
    /// GitHub REST base (`VCSSTAR__GITHUB_API_BASE`), for tests.
    pub github_api_base: String,
    /// Directory with the built web UI (`VCSSTAR__UI_DIR`). Unset or
    /// missing: the API serves without the SPA.
    pub ui_dir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct LinkKeysConfig {
    /// `host:port` of the domain linkkeys server's TCP CSIL port (in-cluster).
    pub rp_addr: Option<String>,
    /// Raw Ed25519 SPKI SHA-256 fingerprints, comma separated.
    pub rp_fingerprints: Vec<String>,
    /// The RP server's own domain; assertions are audience-checked against it.
    pub rp_domain: Option<String>,
    /// API key for the `Rp` service (`SECRET_VCSSTAR__LINKKEYS_API_KEY`).
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
    env::var(format!("VCSSTAR__{name}"))
        .ok()
        .filter(|v| !v.is_empty())
}

fn secret(name: &str) -> Option<String> {
    env::var(format!("SECRET_VCSSTAR__{name}"))
        .ok()
        .filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = var("DATABASE_URL")
            .or_else(|| secret("DATABASE_URL"))
            .or_else(|| env::var("DATABASE_URL").ok())
            .ok_or("VCSSTAR__DATABASE_URL is required")?;
        let instance_domain = var("INSTANCE_DOMAIN").unwrap_or_else(|| "localhost".into());
        let repo_cache_dir = var("REPO_CACHE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("var/repos"));
        let object_store = match (
            var("OBJECT_STORE_ENDPOINT"),
            var("OBJECT_STORE_BUCKET"),
            var("OBJECT_STORE_ACCESS_KEY").or_else(|| secret("OBJECT_STORE_ACCESS_KEY")),
            var("OBJECT_STORE_SECRET_KEY").or_else(|| secret("OBJECT_STORE_SECRET_KEY")),
        ) {
            (Some(endpoint), Some(bucket), Some(access_key), Some(secret_key)) => {
                Some(ObjectStoreConfig {
                    endpoint,
                    bucket,
                    access_key,
                    secret_key,
                    region: var("OBJECT_STORE_REGION").unwrap_or_else(|| "us-east-1".into()),
                })
            }
            (Some(_), Some(_), _, _) => {
                tracing::warn!(
                    "object store endpoint and bucket are set but the access keys are not; history stays on local disk"
                );
                None
            }
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
                .unwrap_or_else(|| format!("vcsstar@{instance_domain}")),
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
            hook_socket: var("HOOK_SOCKET")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| repo_cache_dir.join("hook.sock")),
            hook_command: var("HOOK_COMMAND").unwrap_or_else(|| {
                let exe = std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "vcsstard".into());
                format!("{exe} hook")
            }),
            listen_ssh: var("LISTEN_SSH").unwrap_or_else(|| "127.0.0.1:2222".into()),
            ssh_host_key_path: var("SSH_HOST_KEY_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| repo_cache_dir.join("ssh_host_ed25519_key")),
            ssh_public_host: var("SSH_PUBLIC_HOST"),
            credential_key: secret("CREDENTIAL_KEY").and_then(|v| {
                use base64::Engine;
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(v.trim())
                    .ok()?;
                <[u8; 32]>::try_from(raw.as_slice()).ok()
            }),
            github_api_base: var("GITHUB_API_BASE")
                .unwrap_or_else(|| "https://api.github.com".into()),
            ui_dir: var("UI_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    let p = std::path::PathBuf::from("ui/dist");
                    p.join("index.html").exists().then_some(p)
                })
                .filter(|p| p.join("index.html").exists()),
            repo_cache_dir,
        })
    }
}
