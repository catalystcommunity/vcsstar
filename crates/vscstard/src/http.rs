//! Rocket mount: the CSIL-RPC envelope-in-body carrier, health, and
//! readiness. Rocket runs on its own thread with its own runtime; handlers
//! are synchronous.

use std::sync::Arc;

use rocket::http::{ContentType, Status as HttpStatus};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::{get, post, routes, State};
use vscstar_core::App;
use vscstar_transport::server::handle_envelope;
use vscstar_transport::{Carrier, CredentialSource, Dispatcher, Peer};

use crate::shutdown;

struct Shared {
    app: App,
    dispatcher: Arc<dyn Dispatcher>,
    shutdown: shutdown::State,
}

/// Carrier facts pulled from the HTTP request.
struct HttpPeer(Peer);

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
            if req.headers().get_one("X-Vscstar-Csrf").is_some() {
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
        .merge(("limits.bytes", vscstar_transport::MAX_FRAME as u64))
        .merge(("shutdown.ctrlc", false))
        .merge(("log_level", "off"));
    let app_state = app.clone();
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
        let rocket = rocket::custom(figment)
            .manage(shared)
            .manage(app_state)
            .mount("/", routes![rpc, healthz, readyz])
            .mount("/", crate::auth_pages::routes())
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
