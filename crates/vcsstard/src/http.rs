//! Rocket mount: the CSIL-RPC envelope-in-body carrier, health, and
//! readiness. Rocket runs on its own thread with its own runtime; handlers
//! are synchronous.

use std::sync::Arc;

use rocket::http::{ContentType, Status as HttpStatus};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::{get, post, routes, State};
use vcsstar_core::App;
use vcsstar_transport::server::handle_envelope;
use vcsstar_transport::{Carrier, CredentialSource, Dispatcher, Peer};

use crate::shutdown;

struct Shared {
    app: App,
    dispatcher: Arc<dyn Dispatcher>,
    shutdown: shutdown::State,
}

/// Carrier facts pulled from the HTTP request.
pub(crate) struct HttpPeer(pub(crate) Peer);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for HttpPeer {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let mut peer = Peer {
            carrier: Carrier::Http,
            ..Peer::default()
        };
        peer.remote = req
            .headers()
            .get_one("X-Forwarded-For")
            .map(|s| s.split(',').next().unwrap_or("").trim().to_string())
            .or_else(|| req.client_ip().map(|ip| ip.to_string()))
            .unwrap_or_default();
        if let Some(h) = req.headers().get_one("Authorization") {
            peer.credential = Some(h.to_string());
            peer.credential_source = CredentialSource::Header;
        } else if let Some(c) = req.cookies().get(crate::auth_pages::SESSION_COOKIE) {
            // Cookie credentials need a same-origin check; browsers send the
            // custom header below only from our own origin.
            if req.headers().get_one("X-Vcsstar-Csrf").is_some() {
                peer.credential = Some(c.value().to_string());
                peer.credential_source = CredentialSource::Cookie;
            }
        }
        Outcome::Success(HttpPeer(peer))
    }
}

#[post("/csil/v1/rpc", data = "<body>")]
fn rpc(shared: &State<Shared>, peer: HttpPeer, body: Vec<u8>) -> (ContentType, Vec<u8>) {
    let ct = ContentType::new("application", "cbor");
    (
        ct,
        handle_envelope(shared.dispatcher.as_ref(), &body, peer.0),
    )
}

#[get("/healthz")]
fn healthz() -> &'static str {
    "ok"
}

/// Prometheus text exposition for this process.
#[get("/metrics")]
fn metrics(app: &State<App>) -> (ContentType, String) {
    // Backlog gauges come from Postgres at scrape time, so every node
    // reports the same numbers and no worker has to be the one scraped.
    if let Ok(mut conn) = app.conn() {
        if let Err(e) = vcsstar_core::metrics::refresh_backlog(&mut conn) {
            tracing::warn!(error = %e, "backlog gauges not refreshed");
        }
    }
    (
        ContentType::new("text", "plain; version=0.0.4"),
        vcsstar_core::metrics::render(),
    )
}

/// Client-side routes of the SPA: anything no other route claimed gets
/// index.html. Git paths carry `.git` and match their own routes first.
#[get("/<path..>", rank = 40)]
async fn spa_fallback(app: &State<App>, path: std::path::PathBuf) -> Option<rocket::fs::NamedFile> {
    let dir = app.config.ui_dir.as_ref()?;
    if path.components().count() > 0 && path.to_string_lossy().contains(".git") {
        return None;
    }
    rocket::fs::NamedFile::open(dir.join("index.html"))
        .await
        .ok()
}

/// GitHub webhook receiver. Ingest-first: authenticate the sender, capture
/// the raw item into corndogs, acknowledge. No processing happens here.
#[post("/hooks/github", data = "<body>")]
fn github_hook(shared: &State<Shared>, req: GithubHeaders, body: Vec<u8>) -> (HttpStatus, String) {
    use diesel::prelude::*;
    use vcsstar_db::schema::{credentials, mirror_state, repos};
    let app = &shared.app;
    let metric = |outcome: &str| vcsstar_core::metrics::github_webhook(&req.event, outcome);
    let Some(sig) = req.signature.as_deref() else {
        metric("unauthorized");
        return (
            HttpStatus::Unauthorized,
            "missing X-Hub-Signature-256\n".into(),
        );
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (HttpStatus::BadRequest, "body is not JSON\n".into()),
    };
    let Some(full_name) = parsed
        .pointer("/repository/full_name")
        .and_then(|v| v.as_str())
    else {
        // `ping` for an org-level hook carries no repository; accept it.
        if req.event == "ping" {
            return (HttpStatus::Accepted, "pong\n".into());
        }
        return (
            HttpStatus::BadRequest,
            "payload without repository.full_name\n".into(),
        );
    };
    let mut conn = match app.conn() {
        Ok(c) => c,
        Err(e) => return (HttpStatus::ServiceUnavailable, format!("{}\n", e.message)),
    };
    let target: Option<(uuid::Uuid, uuid::Uuid)> = mirror_state::table
        .inner_join(repos::table)
        .filter(mirror_state::github_repo.eq(full_name))
        .select((repos::id, repos::org_id))
        .first(&mut conn)
        .optional()
        .unwrap_or(None);
    let Some((repo_id, org_id)) = target else {
        metric("unknown_repo");
        return (
            HttpStatus::NotFound,
            "no mirror for this repository\n".into(),
        );
    };
    // Several webhook secrets may be live at once (rotation, or hooks set
    // up at different times); the signature must match one of them.
    let set =
        match vcsstar_core::mirror::cred::candidates(&mut conn, org_id, repo_id, "webhook_inbound")
        {
            Ok(set) if !set.is_empty() => set,
            _ => {
                return (
                    HttpStatus::Unauthorized,
                    "no webhook_inbound credential for this repository\n".into(),
                )
            }
        };
    let mut matched = false;
    for cred in &set {
        let secret = match vcsstar_core::mirror::cred::value(app, cred) {
            Ok(s) => s,
            Err(e) => return (HttpStatus::ServiceUnavailable, format!("{}\n", e.message)),
        };
        if vcsstar_core::github::verify_webhook_signature(secret.as_bytes(), &body, sig) {
            matched = true;
            break;
        }
    }
    if !matched {
        metric("unauthorized");
        return (HttpStatus::Unauthorized, "signature mismatch\n".into());
    }
    let _ = credentials::table; // (schema import kept for clarity)
    let item = vcsstar_workers::ingest::GithubInbound {
        delivery_id: req
            .delivery
            .clone()
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
        event: req.event.clone(),
        body,
        received_at_ms: vcsstar_core::now_ms(),
    };
    let mut payload = Vec::new();
    if ciborium::ser::into_writer(&item, &mut payload).is_err() {
        return (HttpStatus::InternalServerError, "encode failed\n".into());
    }
    match app.submit_task(
        vcsstar_workers::ingest::QUEUE,
        &format!("github:{}", item.delivery_id),
        payload,
        5,
    ) {
        Ok(()) => {
            metric("captured");
            (HttpStatus::Accepted, "captured\n".into())
        }
        // A GitHub App hook and a repository hook deliver one event with the
        // same delivery id and different bodies (the App's carries the
        // installation); the second is the same work and is acknowledged.
        Err(e) if e.code == vcsstar_core::Code::AlreadyExists => {
            metric("duplicate");
            (HttpStatus::Accepted, "duplicate\n".into())
        }
        Err(e) => {
            tracing::warn!(delivery = %item.delivery_id, event = %item.event, error = %e.message, "github webhook not captured");
            metric("rejected");
            (HttpStatus::ServiceUnavailable, format!("{}\n", e.message))
        }
    }
}

struct GithubHeaders {
    event: String,
    delivery: Option<String>,
    signature: Option<String>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GithubHeaders {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        Outcome::Success(GithubHeaders {
            event: req
                .headers()
                .get_one("X-GitHub-Event")
                .unwrap_or("unknown")
                .to_string(),
            delivery: req
                .headers()
                .get_one("X-GitHub-Delivery")
                .map(str::to_string),
            signature: req
                .headers()
                .get_one("X-Hub-Signature-256")
                .map(str::to_string),
        })
    }
}

#[get("/readyz")]
fn readyz(shared: &State<Shared>) -> (HttpStatus, &'static str) {
    if shared.shutdown.stopping() {
        return (HttpStatus::ServiceUnavailable, "stopping");
    }
    match shared.app.db.get() {
        Ok(_) => (HttpStatus::Ok, "ready"),
        Err(_) => (HttpStatus::ServiceUnavailable, "database unavailable"),
    }
}

pub fn serve(
    app: App,
    dispatcher: Arc<dyn Dispatcher>,
    shutdown: shutdown::State,
) -> anyhow::Result<()> {
    let (host, port) = app
        .config
        .listen_http
        .rsplit_once(':')
        .map(|(h, p)| (h.to_string(), p.parse::<u16>().unwrap_or(8080)))
        .unwrap_or_else(|| ("127.0.0.1".into(), 8080));
    let figment = rocket::Config::figment()
        .merge(("address", host))
        .merge(("port", port))
        .merge(("limits.bytes", vcsstar_transport::MAX_FRAME as u64))
        .merge(("shutdown.ctrlc", false))
        .merge(("log_level", "off"));
    let app_state = app.clone();
    let app_ui_dir = app.config.ui_dir.clone();
    let shared = Shared {
        app,
        dispatcher,
        shutdown: shutdown.clone(),
    };
    let rt = rocket::tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let ui_dir = app_ui_dir.clone();
        let mut rocket = rocket::custom(figment).manage(shared).manage(app_state);
        if let Some(dir) = &ui_dir {
            tracing::info!(dir = %dir.display(), "serving the web UI");
            rocket = rocket
                .mount("/", rocket::fs::FileServer::from(dir).rank(30))
                .mount("/", routes![spa_fallback]);
        }
        let rocket = rocket
            .mount("/", routes![rpc, healthz, readyz, metrics, github_hook])
            .mount("/", crate::auth_pages::routes())
            .mount("/", crate::gitwire::routes())
            .mount("/", crate::events_ws::routes())
            .ignite()
            .await?;
        let handle = rocket.shutdown();
        rocket::tokio::spawn(async move {
            loop {
                if shutdown.stopping() {
                    handle.notify();
                    break;
                }
                rocket::tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
        rocket.launch().await?;
        Ok::<(), rocket::Error>(())
    })?;
    Ok(())
}
