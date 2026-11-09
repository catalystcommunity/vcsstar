//! vscstar server binary. One binary, role flags per deployment.

mod auth_pages;
mod http;
mod shutdown;

use std::net::TcpListener;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use vscstar_core::dispatch::AppDispatcher;
use vscstar_core::{App, Config};

#[derive(Parser)]
#[command(name = "vscstard", version, about = "vscstar server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run migrations and serve the selected roles.
    Serve {
        /// Comma-separated roles: api, dispatcher, sweeper, mirror-worker, hook-worker, ingest-worker, maintenance.
        #[arg(long, default_value = "api")]
        roles: String,
    },
    /// Apply pending database migrations and exit.
    Migrate,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .json()
        .init();
    let cli = Cli::parse();
    let config = Config::from_env().map_err(|e| anyhow::anyhow!(e))?;
    let db = vscstar_db::pool(&config.database_url, config.db_pool_size)?;

    match cli.command {
        Command::Migrate => {
            let applied = vscstar_db::run_migrations(&mut *db.get()?)?;
            tracing::info!(count = applied.len(), "migrations applied");
            Ok(())
        }
        Command::BootstrapToken {
            identity,
            handle,
            name,
            scopes,
        } => {
            vscstar_db::run_migrations(&mut *db.get()?)?;
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
        Command::Serve { roles } => {
            vscstar_db::run_migrations(&mut *db.get()?)?;
            let app = App::new(config, db);
            let roles: Vec<String> = roles.split(',').map(|s| s.trim().to_string()).collect();
            serve(app, &roles)
        }
    }
}

mod bootstrap {
    use diesel::prelude::*;
    use vscstar_core::services::parse_identity_key;
    use vscstar_core::tokens::{self, API_TOKEN_PREFIX};
    use vscstar_core::App;
    use vscstar_db::models::{Identity, NewApiToken, NewIdentity};
    use vscstar_db::schema::{api_tokens, identities};

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
                    id: vscstar_db::new_id(),
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
            id: vscstar_db::new_id(),
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
    let dispatcher: Arc<dyn vscstar_transport::Dispatcher> =
        Arc::new(AppDispatcher::new(app.clone()));
    let mut handles = Vec::new();

    if roles.iter().any(|r| r == "api") {
        let listener = TcpListener::bind(&app.config.listen_rpc)?;
        tracing::info!(addr = %app.config.listen_rpc, "csil-rpc tcp listening");
        let d = Arc::clone(&dispatcher);
        let flag = state.flag();
        handles.push(
            std::thread::Builder::new()
                .name("rpc-tcp".into())
                .spawn(move || {
                    if let Err(e) = vscstar_transport::server::serve_tcp(listener, d, flag) {
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
                    if let Err(e) = http::serve(app_http, d, state_http) {
                        tracing::error!(error = %e, "http server stopped");
                    }
                })?,
        );
    }
    for r in roles {
        if !matches!(r.as_str(), "api") {
            tracing::warn!(role = %r, "role not implemented yet; ignored");
        }
    }

    state.wait();
    tracing::info!("shutdown requested; draining");
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}
