//! vcsstar server binary. One binary, role flags per deployment.

mod auth_pages;
mod events_ws;
mod gitwire;
mod http;
mod shutdown;
mod sshd;

use std::net::TcpListener;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use vcsstar_core::dispatch::AppDispatcher;
use vcsstar_core::{App, Config};

#[derive(Parser)]
#[command(name = "vcsstard", version, about = "vcsstar server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run migrations and serve the selected roles.
    Serve {
        /// Comma-separated roles: api, git-ssh, dispatcher, sweeper, mirror-worker, hook-worker, ingest-worker, repack-worker.
        #[arg(long, default_value = "api")]
        roles: String,
    },
    /// Apply pending database migrations and exit.
    Migrate,
    /// Git hook entry point (exec'd by git inside a push). Forwards the ref
    /// updates to the serving process over the hook socket.
    Hook {
        /// Hook name: pre-receive.
        name: String,
    },
    /// Create an identity row and an API token for bootstrap. Prints the
    /// token once. Intended for the first instance admin and for CI service
    /// accounts before LinkKeys login exists in a deployment.
    BootstrapToken {
        /// `user_id@domain` of the identity.
        #[arg(long)]
        identity: String,
        #[arg(long, default_value = "bootstrap")]
        handle: String,
        #[arg(long, default_value = "bootstrap")]
        name: String,
        /// Comma-separated scopes; defaults to `admin`.
        #[arg(long, default_value = "admin")]
        scopes: String,
    },
}

fn main() -> anyhow::Result<()> {
    // Several dependencies link rustls with different crypto features; pick
    // ring once for the whole process before any TLS connection.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _telemetry = telemetry::init()?;
    let cli = Cli::parse();
    if let Command::Hook { name } = &cli.command {
        std::process::exit(vcsstar_store::hook::client_main(name));
    }
    let config = Config::from_env().map_err(|e| anyhow::anyhow!(e))?;
    let db = vcsstar_db::pool(&config.database_url, config.db_pool_size)?;

    match cli.command {
        Command::Migrate => {
            let applied = vcsstar_db::run_migrations(&mut *db.get()?)?;
            tracing::info!(count = applied.len(), "migrations applied");
            Ok(())
        }
        Command::BootstrapToken {
            identity,
            handle,
            name,
            scopes,
        } => {
            vcsstar_db::run_migrations(&mut *db.get()?)?;
            let app = App::new(config, db);
            let scopes: Vec<String> = scopes
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let secret = bootstrap::create(&app, &identity, &handle, &name, &scopes)?;
            println!("{secret}");
            Ok(())
        }
        Command::Hook { .. } => unreachable!("handled before config load"),
        Command::Serve { roles } => {
            vcsstar_db::run_migrations(&mut *db.get()?)?;
            let app = App::new(config, db);
            let roles: Vec<String> = roles.split(',').map(|s| s.trim().to_string()).collect();
            serve(app, &roles)
        }
    }
}

/// Logs always go to stderr as JSON. Traces go to an OTLP/HTTP endpoint
/// when `VCSSTAR__OTLP_ENDPOINT` is set; the exporter runs on its own
/// thread (no async runtime needed) and flushes on drop.
mod telemetry {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    pub struct Guard(Option<SdkTracerProvider>);

    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(p) = self.0.take() {
                let _ = p.shutdown();
            }
        }
    }

    pub fn init() -> anyhow::Result<Guard> {
        let filter =
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?);
        let fmt = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .json();
        let endpoint = std::env::var("VCSSTAR__OTLP_ENDPOINT")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let Some(endpoint) = endpoint else {
            tracing_subscriber::registry().with(filter).with(fmt).init();
            return Ok(Guard(None));
        };
        let service = std::env::var("VCSSTAR__OTLP_SERVICE_NAME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "vcsstard".into());
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
            .build()?;
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name(service.clone())
                    .build(),
            )
            .build();
        let tracer = provider.tracer("vcsstard");
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt)
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .init();
        tracing::info!(endpoint, service, "OTLP trace export enabled");
        Ok(Guard(Some(provider)))
    }
}

mod bootstrap {
    use diesel::prelude::*;
    use vcsstar_core::services::parse_identity_key;
    use vcsstar_core::tokens::{self, API_TOKEN_PREFIX};
    use vcsstar_core::App;
    use vcsstar_db::models::{Identity, NewApiToken, NewIdentity};
    use vcsstar_db::schema::{api_tokens, identities};

    pub fn create(
        app: &App,
        identity: &str,
        handle: &str,
        name: &str,
        scopes: &[String],
    ) -> anyhow::Result<String> {
        let (user_id, domain) =
            parse_identity_key(identity).map_err(|e| anyhow::anyhow!(e.message))?;
        let mut conn = app.db.get()?;
        let existing: Option<Identity> = identities::table
            .filter(identities::lk_user_id.eq(user_id))
            .filter(identities::lk_domain.eq(&domain))
            .select(Identity::as_select())
            .first(&mut conn)
            .optional()?;
        let id = match existing {
            Some(i) => i.id,
            None => {
                let row = NewIdentity {
                    id: vcsstar_db::new_id(),
                    lk_user_id: user_id,
                    lk_domain: &domain,
                    handle,
                    display_name: None,
                    email: None,
                    email_verified: false,
                };
                diesel::insert_into(identities::table)
                    .values(&row)
                    .execute(&mut conn)?;
                row.id
            }
        };
        let secret = tokens::new_secret(API_TOKEN_PREFIX);
        let row = NewApiToken {
            id: vcsstar_db::new_id(),
            identity_id: id,
            token_hash: tokens::hash(&secret),
            name,
            scopes: scopes.iter().cloned().map(Some).collect(),
            client_label: Some("bootstrap"),
            expires_at: None,
        };
        diesel::insert_into(api_tokens::table)
            .values(&row)
            .execute(&mut conn)?;
        Ok(secret)
    }
}

fn serve(app: App, roles: &[String]) -> anyhow::Result<()> {
    let state = shutdown::install();
    let dispatcher: Arc<dyn vcsstar_transport::Dispatcher> =
        Arc::new(AppDispatcher::new(app.clone()));
    let mut handles = Vec::new();

    let serves_git = roles.iter().any(|r| r == "api" || r == "git-ssh");
    if serves_git {
        std::fs::create_dir_all(&app.config.repo_cache_dir)?;
        gitwire::install_hooks(&app)?;
        handles.push(gitwire::spawn_hook_socket(app.clone(), state.clone())?);
        if app.config.cache_warm {
            // A cold node downloads every repository's history now, in the
            // background, so its first clone is served from disk.
            let app_w = app.clone();
            let state_w = state.clone();
            handles.push(
                std::thread::Builder::new()
                    .name("cache-warm".into())
                    .spawn(move || vcsstar_core::vcs::warm_loop(&app_w, || state_w.stopping()))?,
            );
        }
    }
    if roles.iter().any(|r| r == "git-ssh") {
        let app_ssh = app.clone();
        let state_ssh = state.clone();
        handles.push(
            std::thread::Builder::new()
                .name("git-ssh".into())
                .spawn(move || {
                    if let Err(e) = sshd::serve(app_ssh, state_ssh.clone()) {
                        tracing::error!(error = %e, "ssh server stopped");
                        state_ssh.request_stop();
                    }
                })?,
        );
    }

    if roles.iter().any(|r| r == "api") {
        // Instance application keys: enroll once, renew below half-life.
        let app_keys = app.clone();
        let state_keys = state.clone();
        handles.push(
            std::thread::Builder::new()
                .name("instance-keys".into())
                .spawn(move || {
                    let mut next = std::time::Instant::now();
                    while !state_keys.stopping() {
                        if std::time::Instant::now() >= next {
                            match app_keys.conn() {
                                Ok(mut conn) => {
                                    match vcsstar_core::instance_keys::ensure_enrolled(&app_keys, &mut conn) {
                                        Ok(n) if n > 0 => tracing::info!(keys = n, "instance keys enrolled"),
                                        Ok(_) => {}
                                        Err(e) => tracing::warn!(error = %e.message, "instance key enrollment skipped"),
                                    }
                                    match vcsstar_core::instance_keys::renew(&app_keys, &mut conn) {
                                        Ok(n) if n > 0 => tracing::info!(keys = n, "instance key attestations renewed"),
                                        Ok(_) => {}
                                        Err(e) => tracing::warn!(error = %e.message, "instance key renewal failed"),
                                    }
                                }
                                Err(e) => tracing::warn!(error = %e.message, "instance keys: no database"),
                            }
                            next = std::time::Instant::now() + std::time::Duration::from_secs(6 * 3600);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                })?,
        );
        let listener = TcpListener::bind(&app.config.listen_rpc)?;
        tracing::info!(addr = %app.config.listen_rpc, "csil-rpc tcp listening");
        let d = Arc::clone(&dispatcher);
        let flag = state.flag();
        handles.push(
            std::thread::Builder::new()
                .name("rpc-tcp".into())
                .spawn(move || {
                    if let Err(e) = vcsstar_transport::server::serve_tcp(listener, d, flag) {
                        tracing::error!(error = %e, "tcp listener stopped");
                    }
                })?,
        );
        let d = Arc::clone(&dispatcher);
        let app_http = app.clone();
        let state_http = state.clone();
        handles.push(
            std::thread::Builder::new()
                .name("rocket".into())
                .spawn(move || {
                    if let Err(e) = http::serve(app_http, d, state_http.clone()) {
                        tracing::error!(error = %e, "http server stopped");
                        state_http.request_stop();
                    }
                })?,
        );
    }
    for r in roles {
        match r.as_str() {
            "api" | "git-ssh" => {}
            "dispatcher" | "sweeper" | "hook-worker" | "mirror-worker" | "ingest-worker"
            | "repack-worker" => {
                let app_w = app.clone();
                let state_w = state.clone();
                let role = r.clone();
                handles.push(
                    std::thread::Builder::new()
                        .name(role.clone())
                        .spawn(move || {
                            if let Err(e) = vcsstar_workers::run_role(app_w, &role, state_w.flag())
                            {
                                tracing::error!(role = %role, error = %e, "worker role stopped");
                                state_w.request_stop();
                            }
                        })?,
                );
            }
            other => tracing::warn!(role = %other, "unknown role; ignored"),
        }
    }

    state.wait();
    tracing::info!("shutdown requested; draining");
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}
