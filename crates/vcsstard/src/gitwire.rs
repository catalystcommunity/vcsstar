//! Git smart HTTP: `/{org}/{repo}.git/info/refs` and the two service
//! endpoints, wrapping `git upload-pack` / `git receive-pack` in
//! stateless-RPC mode against the hydrated cache directory. Reads on public
//! repos are anonymous; everything else authenticates with an API token as
//! the Basic-auth password (any user name) or a Bearer header.

use std::io::{Read, Write};

use rocket::data::ToByteUnit;
use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::Responder;
use rocket::{get, post, routes, Data, Route, State};
use vcsstar_core::authz::{self, RepoRole};
use vcsstar_core::principal::{self, ReqCtx};
use vcsstar_core::push::{self, PushCtx};
use vcsstar_core::{vcs, App, Code};
use vcsstar_transport::{Carrier, CredentialSource, Peer};
use vcsstar_vcs::{Env, WireService};

const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub fn routes() -> Vec<Route> {
    routes![info_refs, upload_pack, receive_pack]
}

/// Credential from `Authorization: Basic` (token as password) or Bearer.
struct GitPeer {
    peer: Peer,
    protocol: Option<String>,
    gzip: bool,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GitPeer {
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
            if let Some(b64) = h.strip_prefix("Basic ") {
                use base64::Engine;
                if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                    let s = String::from_utf8_lossy(&raw).into_owned();
                    let password = s.split_once(':').map(|(_, p)| p).unwrap_or(&s);
                    if !password.is_empty() {
                        peer.credential = Some(password.to_string());
                        peer.credential_source = CredentialSource::Header;
                    }
                }
            } else {
                peer.credential = Some(h.to_string());
                peer.credential_source = CredentialSource::Header;
            }
        }
        Outcome::Success(GitPeer {
            peer,
            protocol: req.headers().get_one("Git-Protocol").map(|s| s.to_string()),
            gzip: req
                .headers()
                .get_one("Content-Encoding")
                .map(|e| e.contains("gzip"))
                .unwrap_or(false),
        })
    }
}

pub struct GitResponse {
    status: Status,
    content_type: String,
    body: Vec<u8>,
    auth_challenge: bool,
}

impl<'r> Responder<'r, 'static> for GitResponse {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut b = rocket::Response::build();
        b.status(self.status)
            .header(ContentType::parse_flexible(&self.content_type).unwrap_or(ContentType::Binary))
            .header(Header::new("Cache-Control", "no-cache"))
            .header(Header::new("Pragma", "no-cache"))
            .header(Header::new("Expires", "Fri, 01 Jan 1980 00:00:00 GMT"));
        if self.auth_challenge {
            b.header(Header::new("WWW-Authenticate", "Basic realm=\"vcsstar\""));
        }
        b.sized_body(self.body.len(), std::io::Cursor::new(self.body));
        Ok(b.finalize())
    }
}

fn text(status: Status, msg: &str, auth_challenge: bool) -> GitResponse {
    GitResponse {
        status,
        content_type: "text/plain; charset=utf-8".into(),
        body: format!("{msg}\n").into_bytes(),
        auth_challenge,
    }
}

fn pkt_line(s: &str) -> Vec<u8> {
    let mut out = format!("{:04x}", s.len() + 4).into_bytes();
    out.extend_from_slice(s.as_bytes());
    out
}

struct Opened {
    org: vcsstar_db::models::Org,
    repo: vcsstar_db::models::Repo,
    role: RepoRole,
    dir: std::path::PathBuf,
    ctx: ReqCtx,
}

/// Resolve, authorize, hydrate. `None` with a response when refused.
fn open_repo(
    app: &App,
    peer: &Peer,
    org: &str,
    repo_seg: &str,
    svc: WireService,
) -> Result<Opened, GitResponse> {
    let name = repo_seg.strip_suffix(".git").unwrap_or(repo_seg);
    let principal = match principal::resolve(app, peer) {
        Ok(p) => p,
        Err(e) if e.code == Code::Unauthenticated || e.code == Code::TokenExpired => {
            return Err(text(Status::Unauthorized, &e.message, true));
        }
        Err(e) => return Err(text(Status::Forbidden, &e.message, false)),
    };
    let anonymous = principal.is_none();
    let ctx = ReqCtx {
        request_id: uuid::Uuid::now_v7().to_string(),
        principal,
        peer: peer.clone(),
    };
    let mut conn = match app.conn() {
        Ok(c) => c,
        Err(e) => return Err(text(Status::ServiceUnavailable, &e.message, false)),
    };
    let (org_row, repo, role) = match authz::load_repo_visible(app, &mut conn, &ctx, org, name) {
        Ok(v) => v,
        Err(e) if e.code == Code::NotFound && anonymous => {
            // The repo may exist and be private: ask for credentials rather
            // than confirm or deny existence.
            return Err(text(Status::Unauthorized, "authentication required", true));
        }
        Err(e) if e.code == Code::NotFound => {
            return Err(text(Status::NotFound, "repository not found", false))
        }
        Err(e) => return Err(text(Status::InternalServerError, &e.message, false)),
    };
    let needed = match svc {
        WireService::UploadPack => RepoRole::Read,
        WireService::ReceivePack => RepoRole::Write,
    };
    if role < needed {
        if anonymous {
            return Err(text(Status::Unauthorized, "authentication required", true));
        }
        return Err(text(Status::Forbidden, "write access required", false));
    }
    if svc == WireService::ReceivePack {
        if let Some(p) = &ctx.principal {
            if !p.has_scope("repo:write") {
                return Err(text(
                    Status::Forbidden,
                    "credential lacks scope repo:write",
                    false,
                ));
            }
        }
        if repo.archived {
            return Err(text(Status::Forbidden, "repository is archived", false));
        }
    }
    let dir = match vcs::hydrate(app, &mut conn, &repo) {
        Ok(d) => d,
        Err(e) => return Err(text(Status::InternalServerError, &e.message, false)),
    };
    Ok(Opened {
        org: org_row,
        repo,
        role,
        dir,
        ctx,
    })
}

fn protocol_env(protocol: &Option<String>) -> Env {
    let mut env = Env::new();
    if let Some(p) = protocol {
        env.push(("GIT_PROTOCOL".into(), p.clone()));
    }
    env
}

/// Run a git service with the whole request body on stdin and collect stdout.
fn run_service(
    app: &App,
    dir: &std::path::Path,
    svc: WireService,
    advertise: bool,
    env: Env,
    body: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let mut child = app
        .store
        .backend()
        .spawn_service(dir, svc, true, advertise, &env)
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin.take() {
            let _ = stdin.write_all(&body);
        }
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stderr.read_to_end(&mut s);
        s
    });
    let mut out = Vec::new();
    stdout.read_to_end(&mut out).map_err(|e| e.to_string())?;
    let _ = writer.join();
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err_reader.join().unwrap_or_default();
    if !status.success() && out.is_empty() {
        return Err(String::from_utf8_lossy(&err).into_owned());
    }
    if !err.is_empty() {
        tracing::debug!(stderr = %String::from_utf8_lossy(&err), "git service stderr");
    }
    Ok(out)
}

#[get("/<org>/<repo>/info/refs?<service>")]
async fn info_refs(
    app: &State<App>,
    peer: GitPeer,
    org: &str,
    repo: &str,
    service: Option<&str>,
) -> GitResponse {
    let Some(svc) = service.and_then(WireService::parse) else {
        return text(
            Status::Forbidden,
            "dumb HTTP is not supported; use a smart git client",
            false,
        );
    };
    let app = app.inner().clone();
    let org = org.to_string();
    let repo = repo.to_string();
    rocket::tokio::task::spawn_blocking(move || {
        let opened = match open_repo(&app, &peer.peer, &org, &repo, svc) {
            Ok(o) => o,
            Err(r) => return r,
        };
        let mut env = protocol_env(&peer.protocol);
        if svc == WireService::ReceivePack {
            // Advertisement only; the push context comes with the POST.
            env.push((vcsstar_store::hook::ENV_PUSH_ID.into(), String::new()));
        }
        match run_service(&app, &opened.dir, svc, true, env, Vec::new()) {
            Ok(adv) => {
                let mut body = pkt_line(&format!("# service={}\n", svc.as_str()));
                body.extend_from_slice(b"0000");
                body.extend_from_slice(&adv);
                GitResponse {
                    status: Status::Ok,
                    content_type: format!("application/x-{}-advertisement", svc.as_str()),
                    body,
                    auth_challenge: false,
                }
            }
            Err(e) => text(Status::InternalServerError, &e, false),
        }
    })
    .await
    .unwrap_or_else(|_| text(Status::InternalServerError, "service task failed", false))
}

async fn read_body(data: Data<'_>, gzip: bool) -> Result<Vec<u8>, GitResponse> {
    let raw = data
        .open(MAX_BODY_BYTES.bytes())
        .into_bytes()
        .await
        .map_err(|e| text(Status::BadRequest, &e.to_string(), false))?;
    if !raw.is_complete() {
        return Err(text(
            Status::PayloadTooLarge,
            "request body too large",
            false,
        ));
    }
    let raw = raw.into_inner();
    if gzip {
        let mut d = flate2::read::GzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        d.read_to_end(&mut out)
            .map_err(|e| text(Status::BadRequest, &format!("gzip: {e}"), false))?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

#[post("/<org>/<repo>/git-upload-pack", data = "<data>")]
async fn upload_pack(
    app: &State<App>,
    peer: GitPeer,
    org: &str,
    repo: &str,
    data: Data<'_>,
) -> GitResponse {
    let body = match read_body(data, peer.gzip).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let app = app.inner().clone();
    let org = org.to_string();
    let repo = repo.to_string();
    rocket::tokio::task::spawn_blocking(move || {
        let opened = match open_repo(&app, &peer.peer, &org, &repo, WireService::UploadPack) {
            Ok(o) => o,
            Err(r) => return r,
        };
        let env = protocol_env(&peer.protocol);
        match run_service(&app, &opened.dir, WireService::UploadPack, false, env, body) {
            Ok(out) => GitResponse {
                status: Status::Ok,
                content_type: "application/x-git-upload-pack-result".into(),
                body: out,
                auth_challenge: false,
            },
            Err(e) => text(Status::InternalServerError, &e, false),
        }
    })
    .await
    .unwrap_or_else(|_| text(Status::InternalServerError, "service task failed", false))
}

#[post("/<org>/<repo>/git-receive-pack", data = "<data>")]
async fn receive_pack(
    app: &State<App>,
    peer: GitPeer,
    org: &str,
    repo: &str,
    data: Data<'_>,
) -> GitResponse {
    let body = match read_body(data, peer.gzip).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let app = app.inner().clone();
    let org = org.to_string();
    let repo = repo.to_string();
    rocket::tokio::task::spawn_blocking(move || {
        let opened = match open_repo(&app, &peer.peer, &org, &repo, WireService::ReceivePack) {
            Ok(o) => o,
            Err(r) => return r,
        };
        let Some(principal) = opened.ctx.principal.clone() else {
            return text(Status::Unauthorized, "authentication required", true);
        };
        let push_id = uuid::Uuid::now_v7();
        let guard = app.pushes.begin(PushCtx {
            id: push_id,
            org: opened.org.clone(),
            repo: opened.repo.clone(),
            dir: opened.dir.clone(),
            principal,
            role: opened.role,
            request_id: opened.ctx.request_id.clone(),
        });
        let mut env = protocol_env(&peer.protocol);
        env.extend(push::receive_env(&app, push_id));
        let result = run_service(
            &app,
            &opened.dir,
            WireService::ReceivePack,
            false,
            env,
            body,
        );
        drop(guard);
        match result {
            Ok(out) => GitResponse {
                status: Status::Ok,
                content_type: "application/x-git-receive-pack-result".into(),
                body: out,
                auth_challenge: false,
            },
            Err(e) => text(Status::InternalServerError, &e, false),
        }
    })
    .await
    .unwrap_or_else(|_| text(Status::InternalServerError, "service task failed", false))
}

pub fn install_hooks(app: &App) -> anyhow::Result<()> {
    app.store.install_hooks(&app.config.hook_command)?;
    Ok(())
}

pub fn spawn_hook_socket(
    app: App,
    state: crate::shutdown::State,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("hook-socket".into())
        .spawn(move || {
            if let Err(e) = push::serve_hook_socket(app, state.flag()) {
                tracing::error!(error = %e, "hook socket stopped; pushes cannot complete");
                state.request_stop();
            }
        })
}
