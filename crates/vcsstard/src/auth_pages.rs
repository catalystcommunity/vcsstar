//! Server-rendered browser pages for login, the IDP callback, and device
//! approval. The sign-in form is a native POST: the response is a 302 to
//! the IDP, and a fetch would follow it cross-origin and burn the
//! single-use login URL. These pages are deliberately plain; the SolidJS UI
//! arrives in phase 5 and calls the same operations.

#![allow(clippy::result_large_err)]

use rocket::form::Form;
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::response::content::RawHtml;
use rocket::response::Redirect;
use rocket::{get, post, routes, FromForm, Route, State};
use vcsstar_core::principal::{self, ReqCtx};
use vcsstar_core::services::login;
use vcsstar_core::App;
use vcsstar_transport::{Carrier, CredentialSource, Peer};

use crate::http::HttpPeer;

pub const SESSION_COOKIE: &str = "vcsstar_session";
const ATTEMPT_COOKIE: &str = "vcsstar_login_attempt";

fn page(title: &str, body: &str) -> RawHtml<String> {
    RawHtml(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{title} · vcsstar</title><style>body{{font:16px system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;color:#222}}\
input,button{{font:inherit;padding:.5rem .75rem;margin:.25rem 0}}input[type=text]{{width:100%;box-sizing:border-box}}\
button{{cursor:pointer}}.err{{color:#a00}}.muted{{color:#666}}code{{background:#f3f3f3;padding:.1rem .3rem}}</style></head>\
<body><h1>{title}</h1>{body}</body></html>"
    ))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn anon_ctx(remote: &str) -> ReqCtx {
    ReqCtx {
        request_id: uuid::Uuid::now_v7().to_string(),
        principal: None,
        peer: Peer {
            carrier: Carrier::Http,
            remote: remote.to_string(),
            ..Peer::default()
        },
    }
}

fn session_ctx(app: &App, cookies: &CookieJar<'_>, remote: &str) -> ReqCtx {
    let peer = Peer {
        credential: cookies.get(SESSION_COOKIE).map(|c| c.value().to_string()),
        credential_source: CredentialSource::Cookie,
        carrier: Carrier::Http,
        remote: remote.to_string(),
    };
    let principal = principal::resolve(app, &peer).ok().flatten();
    ReqCtx {
        request_id: uuid::Uuid::now_v7().to_string(),
        principal,
        peer,
    }
}

#[derive(FromForm)]
struct LoginForm {
    identity: String,
    return_to: Option<String>,
    device_code: Option<String>,
}

#[get("/auth/login?<return_to>&<device_code>&<error>")]
fn login_form(
    return_to: Option<String>,
    device_code: Option<String>,
    error: Option<String>,
) -> RawHtml<String> {
    let err = error
        .map(|e| format!("<p class=\"err\">{}</p>", esc(&e)))
        .unwrap_or_default();
    let device = device_code
        .as_deref()
        .map(|c| format!("<p>Approving CLI login code <code>{}</code> after sign-in.</p><input type=\"hidden\" name=\"device_code\" value=\"{}\">", esc(c), esc(c)))
        .unwrap_or_default();
    let rt = return_to
        .as_deref()
        .map(|r| {
            format!(
                "<input type=\"hidden\" name=\"return_to\" value=\"{}\">",
                esc(r)
            )
        })
        .unwrap_or_default();
    page(
        "Sign in",
        &format!(
            "{err}<form method=\"post\" action=\"/auth/login\">{device}{rt}\
<label for=\"identity\">Your LinkKeys identity</label>\
<input type=\"text\" id=\"identity\" name=\"identity\" placeholder=\"you@example.com\" autocomplete=\"username\" autofocus required>\
<button type=\"submit\">Continue</button></form>\
<p class=\"muted\">vcsstar has no accounts of its own. Sign in with the LinkKeys identity of any domain.</p>"
        ),
    )
}

#[post("/auth/login", data = "<form>")]
fn login_submit(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    form: Form<LoginForm>,
) -> Result<Redirect, Redirect> {
    let req = vcsstar_types::BeginLoginRequest {
        ctx: vcsstar_types::Ctx {
            request_id: uuid::Uuid::now_v7().to_string(),
            deadline_ms: None,
            contract_version: None,
            contract_digest: None,
        },
        identity_input: form.identity.clone(),
        return_to: form.return_to.clone().filter(|s| !s.is_empty()),
        device_user_code: form.device_code.clone().filter(|s| !s.is_empty()),
    };
    match login::begin(app, &anon_ctx(&peer.0.remote), req) {
        Ok(resp) => {
            let secure = app.config.public_url.starts_with("https://");
            cookies.add(
                Cookie::build((ATTEMPT_COOKIE, resp.attempt_token))
                    .http_only(true)
                    .secure(secure)
                    .same_site(SameSite::Lax)
                    .path("/auth")
                    .max_age(rocket::time::Duration::minutes(login::LOGIN_TTL_MINUTES))
                    .build(),
            );
            Ok(Redirect::to(resp.redirect_url))
        }
        Err(e) => Err(Redirect::to(format!(
            "/auth/login?error={}",
            vcsstar_core::linkkeys::urlencode(&e.message)
        ))),
    }
}

#[get("/auth/callback?<encrypted_token>&<error>")]
fn callback(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    encrypted_token: Option<String>,
    error: Option<String>,
) -> Result<Redirect, Redirect> {
    let attempt = cookies.get(ATTEMPT_COOKIE).map(|c| c.value().to_string());
    cookies.remove(Cookie::build(ATTEMPT_COOKIE).path("/auth"));
    if let Some(e) = error {
        return Err(Redirect::to(format!(
            "/auth/login?error={}",
            vcsstar_core::linkkeys::urlencode(&format!("identity provider: {e}"))
        )));
    }
    let (Some(attempt), Some(token)) = (attempt, encrypted_token) else {
        return Err(Redirect::to(
            "/auth/login?error=login%20attempt%20missing%20or%20expired",
        ));
    };
    let req = vcsstar_types::CompleteLoginRequest {
        ctx: vcsstar_types::Ctx {
            request_id: uuid::Uuid::now_v7().to_string(),
            deadline_ms: None,
            contract_version: None,
            contract_digest: None,
        },
        attempt_token: attempt,
        encrypted_token: token,
    };
    match login::complete(app, &anon_ctx(&peer.0.remote), req) {
        Ok(session) => {
            let secure = app.config.public_url.starts_with("https://");
            cookies.add(
                Cookie::build((SESSION_COOKIE, session.session_token))
                    .http_only(true)
                    .secure(secure)
                    .same_site(SameSite::Lax)
                    .path("/")
                    .max_age(rocket::time::Duration::hours(login::SESSION_SLIDING_HOURS))
                    .build(),
            );
            let target = session
                .return_to
                .filter(|r| r.starts_with('/') && !r.starts_with("//"))
                .unwrap_or_else(|| "/auth/me".to_string());
            Ok(Redirect::to(target))
        }
        Err(e) => Err(Redirect::to(format!(
            "/auth/login?error={}",
            vcsstar_core::linkkeys::urlencode(&e.message)
        ))),
    }
}

#[get("/auth/me")]
fn me(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
) -> Result<RawHtml<String>, Redirect> {
    let ctx = session_ctx(app, cookies, &peer.0.remote);
    let Some(p) = ctx.principal.as_ref() else {
        return Err(Redirect::to("/auth/login?return_to=/auth/me"));
    };
    Ok(page(
        "Signed in",
        &format!(
            "<p>You are <strong>{}@{}</strong> <span class=\"muted\">({})</span>.</p>\
<p><a href=\"/auth/device\">Approve a CLI login</a> · <form method=\"post\" action=\"/auth/logout\" style=\"display:inline\"><button type=\"submit\">Sign out</button></form></p>",
            esc(&p.identity.handle),
            esc(&p.identity.lk_domain),
            esc(&p.key())
        ),
    ))
}

#[post("/auth/logout")]
fn logout(app: &State<App>, cookies: &CookieJar<'_>, peer: HttpPeer) -> Redirect {
    let ctx = session_ctx(app, cookies, &peer.0.remote);
    if ctx.principal.is_some() {
        let _ = vcsstar_core::services::auth::logout(
            app,
            &ctx,
            vcsstar_types::Empty { ctx: empty_ctx() },
        );
    }
    cookies.remove(Cookie::build(SESSION_COOKIE).path("/"));
    Redirect::to("/auth/login")
}

fn empty_ctx() -> vcsstar_types::Ctx {
    vcsstar_types::Ctx {
        request_id: uuid::Uuid::now_v7().to_string(),
        deadline_ms: None,
        contract_version: None,
        contract_digest: None,
    }
}

#[derive(FromForm)]
struct DeviceForm {
    code: String,
    decision: String,
}

#[get("/auth/device?<code>&<msg>")]
fn device_form(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    code: Option<String>,
    msg: Option<String>,
) -> Result<RawHtml<String>, Redirect> {
    let ctx = session_ctx(app, cookies, &peer.0.remote);
    if ctx.principal.is_none() {
        let mut target = "/auth/login?return_to=/auth/device".to_string();
        if let Some(c) = code.as_deref() {
            target = format!(
                "/auth/login?device_code={}",
                vcsstar_core::linkkeys::urlencode(c)
            );
        }
        return Err(Redirect::to(target));
    }
    let note = msg
        .map(|m| format!("<p>{}</p>", esc(&m)))
        .unwrap_or_default();
    Ok(page(
        "Approve CLI login",
        &format!(
            "{note}<form method=\"post\" action=\"/auth/device\">\
<label for=\"code\">Code shown by the CLI</label>\
<input type=\"text\" id=\"code\" name=\"code\" value=\"{}\" autocomplete=\"off\" required>\
<button type=\"submit\" name=\"decision\" value=\"approve\">Approve</button> \
<button type=\"submit\" name=\"decision\" value=\"deny\">Deny</button></form>\
<p class=\"muted\">Only approve a code you requested yourself a moment ago.</p>",
            esc(code.as_deref().unwrap_or(""))
        ),
    ))
}

#[post("/auth/device", data = "<form>")]
fn device_submit(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    form: Form<DeviceForm>,
) -> Redirect {
    let ctx = session_ctx(app, cookies, &peer.0.remote);
    let req = vcsstar_types::DeviceApproveRequest {
        ctx: empty_ctx(),
        user_code: form.code.clone(),
        approve: form.decision == "approve",
    };
    match login::approve_device(app, &ctx, req) {
        Ok(_) => Redirect::to(format!(
            "/auth/device?msg={}",
            if form.decision == "approve" {
                "Approved.%20You%20can%20return%20to%20the%20terminal."
            } else {
                "Denied."
            }
        )),
        Err(e) => Redirect::to(format!(
            "/auth/device?msg={}",
            vcsstar_core::linkkeys::urlencode(&e.message)
        )),
    }
}

fn account_target(return_to: Option<&str>) -> String {
    return_to
        .filter(|r| r.starts_with('/') && !r.starts_with("//"))
        .unwrap_or("/me")
        .to_string()
}

fn link_error(return_to: Option<&str>, message: &str) -> Redirect {
    let target = account_target(return_to);
    let sep = if target.contains('?') { '&' } else { '?' };
    Redirect::to(format!(
        "{target}{sep}github_error={}",
        vcsstar_core::linkkeys::urlencode(message)
    ))
}

/// Start linking the signed-in account to GitHub through the instance's
/// GitHub App: remember the attempt and send the browser to GitHub.
#[get("/auth/github/start?<return_to>")]
fn github_start(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    return_to: Option<String>,
) -> Redirect {
    let ctx = session_ctx(app, cookies, &peer.0.remote);
    let Some(p) = ctx.principal.as_ref() else {
        return Redirect::to(format!(
            "/auth/login?return_to={}",
            vcsstar_core::linkkeys::urlencode(&format!(
                "/auth/github/start?return_to={}",
                account_target(return_to.as_deref())
            ))
        ));
    };
    let mut conn = match app.conn() {
        Ok(c) => c,
        Err(e) => return link_error(return_to.as_deref(), &e.message),
    };
    match vcsstar_core::github_link::begin(app, &mut conn, p.identity.id, return_to.as_deref()) {
        Ok(url) => Redirect::to(url),
        Err(e) => link_error(return_to.as_deref(), &e.message),
    }
}

/// GitHub sends the browser back here with the authorization code. A
/// visit without our `state` (for example right after installing the App
/// with "request user authorization during installation") is sent through
/// `/auth/github/start`, so a link is only ever made for an attempt this
/// account started.
#[get("/auth/github/callback?<code>&<state>&<error>&<error_description>&<setup_action>&<installation_id>")]
#[allow(clippy::too_many_arguments)]
fn github_callback(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    setup_action: Option<String>,
    installation_id: Option<String>,
) -> Redirect {
    let _ = (setup_action, installation_id);
    if let Some(e) = error {
        let d = error_description.unwrap_or_default();
        return link_error(None, &format!("GitHub: {e} {d}"));
    }
    let ctx = session_ctx(app, cookies, &peer.0.remote);
    let Some(p) = ctx.principal.as_ref() else {
        return Redirect::to("/auth/login?return_to=%2Fauth%2Fgithub%2Fstart");
    };
    let (Some(code), Some(state)) = (code, state) else {
        return Redirect::to("/auth/github/start");
    };
    let mut conn = match app.conn() {
        Ok(c) => c,
        Err(e) => return link_error(None, &e.message),
    };
    match vcsstar_core::github_link::complete(app, &mut conn, p.identity.id, &state, &code) {
        Ok((row, return_to)) => {
            tracing::info!(identity = %p.key(), login = %row.login, "GitHub account linked");
            Redirect::to(account_target(return_to.as_deref()))
        }
        Err(e) => link_error(None, &e.message),
    }
}

/// Without a built UI, the root shows the session page. The SPA file
/// server (rank 30) wins when `VCSSTAR__UI_DIR` is set.
#[get("/", rank = 50)]
fn index() -> Redirect {
    Redirect::to("/auth/me")
}

pub fn routes() -> Vec<Route> {
    routes![
        index,
        login_form,
        login_submit,
        callback,
        me,
        logout,
        device_form,
        device_submit,
        github_start,
        github_callback
    ]
}

#[allow(dead_code)]
fn _status_unused() -> Status {
    Status::Ok
}
