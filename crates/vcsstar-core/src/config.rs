//! Environment-first configuration. Prefix `VCSSTAR__` (double underscore, so
//! Kubernetes Service links such as `VCSSTAR_PORT` never match). Secrets use
//! `SECRET_VCSSTAR__` and are never logged.

use std::env;

/// Token-bucket sizes per minute; `0` disables a limit. Buckets are per
/// node and keyed by peer address, typed identity, device code, or
/// approving principal (`VCSSTAR__RATE_*_PER_MINUTE`).
#[derive(Debug, Clone)]
pub struct RateLimits {
    /// `Auth/begin-login`, `Auth/complete-login`, `Auth/begin-device-login`
    /// per peer address, and `begin-login` per typed identity.
    pub login_per_minute: u32,
    /// `Auth/poll-device-login` per device code and per peer address.
    pub device_poll_per_minute: u32,
    /// `Auth/approve-device-login` per approving principal: user codes are
    /// short, so guesses must stay slow.
    pub device_approve_per_minute: u32,
}

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
    /// Per-node limits on the unauthenticated login entry points.
    pub rate_limits: RateLimits,
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
    /// Warm this node's repository cache in the background: download every
    /// repository's history segments and refs after start and again every
    /// `cache_warm_interval` (`VCSSTAR__CACHE_WARM`, default true;
    /// `VCSSTAR__CACHE_WARM_INTERVAL_S`, default 600). A cold pod then
    /// serves its first clone from disk instead of from the object store.
    pub cache_warm: bool,
    pub cache_warm_interval: std::time::Duration,
    /// Repositories with at least this many live history segments are
    /// repacked into one by the `repack-worker` role
    /// (`VCSSTAR__REPACK_MIN_SEGMENTS`, default 20).
    pub repack_min_segments: i64,
    /// GitHub REST base (`VCSSTAR__GITHUB_API_BASE`), for tests.
    pub github_api_base: String,
    /// Base of GitHub git URLs (`VCSSTAR__GITHUB_GIT_BASE`, default
    /// `https://github.com`); a test points it at a directory of bare
    /// repositories with a `file://` URL.
    pub github_git_base: String,
    /// OTLP/HTTP traces endpoint (`VCSSTAR__OTLP_ENDPOINT`), for example
    /// `http://otel-collector:4318`. Unset: no tracing export.
    pub otlp_endpoint: Option<String>,
    /// `service.name` on exported traces (`VCSSTAR__OTLP_SERVICE_NAME`).
    pub otlp_service_name: String,
    /// Directory with the built web UI (`VCSSTAR__UI_DIR`). Unset or
    /// missing: the API serves without the SPA.
    pub ui_dir: Option<std::path::PathBuf>,
    /// Base of GitHub's web pages (`VCSSTAR__GITHUB_WEB_BASE`, default
    /// `https://github.com`): the OAuth authorize and token endpoints.
    pub github_web_base: String,
    /// The instance's GitHub App, when one is registered. It serves two
    /// purposes: installation tokens for mirrored repositories whose owner
    /// installed it, and user authorization so vcsstar acts as a linked
    /// user.
    pub github_app: Option<GithubAppConfig>,
}

#[derive(Debug, Clone)]
pub struct GithubAppConfig {
    /// Numeric App id (`SECRET_VCSSTAR__GITHUB_APP_ID` or `VCSSTAR__GITHUB_APP_ID`).
    pub app_id: String,
    /// OAuth client id of the App (`..._GITHUB_APP_CLIENT_ID`).
    pub client_id: String,
    /// OAuth client secret (`SECRET_VCSSTAR__GITHUB_APP_CLIENT_SECRET`).
    pub client_secret: String,
    /// PEM private key of the App (`SECRET_VCSSTAR__GITHUB_APP_PRIVATE_KEY`).
    pub private_key: String,
    /// Public slug, for display (`VCSSTAR__GITHUB_APP_SLUG`); optional.
    pub slug: Option<String>,
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
            rate_limits: RateLimits {
                login_per_minute: var("RATE_LOGIN_PER_MINUTE")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10),
                device_poll_per_minute: var("RATE_DEVICE_POLL_PER_MINUTE")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(40),
                device_approve_per_minute: var("RATE_DEVICE_APPROVE_PER_MINUTE")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(6),
            },
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
            cache_warm: var("CACHE_WARM").map(|v| v != "false").unwrap_or(true),
            cache_warm_interval: std::time::Duration::from_secs(
                var("CACHE_WARM_INTERVAL_S")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600),
            ),
            repack_min_segments: var("REPACK_MIN_SEGMENTS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            github_api_base: var("GITHUB_API_BASE")
                .unwrap_or_else(|| "https://api.github.com".into()),
            github_git_base: var("GITHUB_GIT_BASE")
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://github.com".into()),
            otlp_endpoint: var("OTLP_ENDPOINT"),
            otlp_service_name: var("OTLP_SERVICE_NAME").unwrap_or_else(|| "vcsstard".into()),
            ui_dir: var("UI_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    let p = std::path::PathBuf::from("ui/dist");
                    p.join("index.html").exists().then_some(p)
                })
                .filter(|p| p.join("index.html").exists()),
            repo_cache_dir,
            github_web_base: var("GITHUB_WEB_BASE")
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://github.com".into()),
            github_app: github_app_from_env(),
        })
    }
}

/// The GitHub App is configured when all four values are present; a
/// partial set is logged and ignored, so a missing Secret key cannot pass
/// as "no App".
fn github_app_from_env() -> Option<GithubAppConfig> {
    let app_id = secret("GITHUB_APP_ID").or_else(|| var("GITHUB_APP_ID"));
    let client_id = secret("GITHUB_APP_CLIENT_ID").or_else(|| var("GITHUB_APP_CLIENT_ID"));
    let client_secret = secret("GITHUB_APP_CLIENT_SECRET");
    let private_key = secret("GITHUB_APP_PRIVATE_KEY");
    match (app_id, client_id, client_secret, private_key) {
        (Some(app_id), Some(client_id), Some(client_secret), Some(private_key)) => {
            Some(GithubAppConfig {
                app_id: app_id.trim().to_string(),
                client_id: client_id.trim().to_string(),
                client_secret: client_secret.trim().to_string(),
                private_key: private_key.replace("\\n", "\n"),
                slug: var("GITHUB_APP_SLUG"),
            })
        }
        (None, None, None, None) => None,
        (a, c, s, k) => {
            tracing::warn!(
                app_id = a.is_some(),
                client_id = c.is_some(),
                client_secret = s.is_some(),
                private_key = k.is_some(),
                "GitHub App configuration is incomplete; the App is ignored"
            );
            None
        }
    }
}
