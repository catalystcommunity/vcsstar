//! Server-rendered browser pages for login, the IDP callback, and device
//! approval. The sign-in form is a native POST: the response is a 302 to
//! the IDP, and a fetch would follow it cross-origin and burn the
//! single-use login URL.

#![allow(clippy::result_large_err)]

use rocket::form::Form;
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::response::content::{RawCss, RawHtml};
use rocket::response::Redirect;
use rocket::{get, post, routes, FromForm, Route, State};
use vcsstar_core::linkkeys::urlencode;
use vcsstar_core::principal::{self, ReqCtx};
use vcsstar_core::services::login;
use vcsstar_core::App;
use vcsstar_transport::{Carrier, CredentialSource, Peer};

use crate::http::HttpPeer;

pub const SESSION_COOKIE: &str = "vcsstar_session";
const CSRF_COOKIE: &str = "vcsstar_login_csrf";
const ATTEMPT_COOKIE: &str = "vcsstar_login_attempt";

#[get("/auth/style.css")]
fn auth_style() -> RawCss<&'static str> {
    RawCss(concat!(
        include_str!("../../../ui/src/shell.css"),
        include_str!("auth.css")
    ))
}

fn page(title: &str, body: &str) -> RawHtml<String> {
    let logo = include_str!("../../../ui/public/logo.svg");
    RawHtml(format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="dark">
<title>{title} · vcsstar</title><link rel="stylesheet" href="/auth/style.css"><link rel="icon" href="/favicon.svg"></head>
<body><a class="skip-link" href="#main-content">Skip to content</a>
<header class="site-header"><a class="brand" href="/"><span class="brand-mark" aria-hidden="true">{logo}</span>vcsstar</a>
<nav aria-label="Main"><a href="/">Repositories</a></nav></header>
<main id="main-content"><h1>{title}</h1>{body}</main></body></html>"##
    ))
}

fn destination(return_to: Option<&str>, device_code: Option<&str>) -> String {
    if let Some(code) = device_code.filter(|s| !s.is_empty()) {
        return format!("/auth/device?code={}", urlencode(code));
    }
    return_to
        .filter(|s| login::check_return_to(Some(s)).is_ok())
        .unwrap_or("/")
        .into()
}

fn login_url(target: &str, link_local: bool) -> String {
    let mut url = format!("/auth/login?return_to={}", urlencode(target));
    if link_local {
        url.push_str("&link_local=true");
    }
    url
}

fn retry(url: &str, message: &str) -> Redirect {
    Redirect::to(format!("{url}&error={}", urlencode(message)))
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
    csrf: String,
    link_local: Option<bool>,
    identity: String,
    return_to: Option<String>,
    device_code: Option<String>,
}

#[get("/auth/login?<return_to>&<device_code>&<error>&<link_local>")]
fn login_form(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    link_local: Option<bool>,
    return_to: Option<String>,
    device_code: Option<String>,
    error: Option<String>,
) -> RawHtml<String> {
    let csrf = vcsstar_core::tokens::new_secret("csrf_");
    cookies.add(
        Cookie::build((CSRF_COOKIE, csrf.clone()))
            .http_only(true)
            .secure(app.config.public_url.starts_with("https://"))
            .same_site(SameSite::Strict)
            .path("/auth")
            .build(),
    );
    let csrf_field = format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        esc(&csrf)
    );
    let link_field = if link_local.unwrap_or(false) {
        "<input type=\"hidden\" name=\"link_local\" value=\"true\">"
    } else {
        ""
    };
    let err = error
        .map(|e| format!("<p class=\"err\" role=\"alert\">{}</p>", esc(&e)))
        .unwrap_or_default();
    let target = destination(return_to.as_deref(), device_code.as_deref());
    let rt = format!(
        r#"<input type="hidden" name="return_to" value="{}">"#,
        esc(&target)
    );
    let linking = link_local.unwrap_or(false);
    let local = if linking {
        String::new()
    } else {
        format!(
            r#"
<section><h2>Local account</h2><form method="post" action="/auth/local">{csrf_field}{rt}
<label for="local-user">Username</label><input type="text" id="local-user" name="username" autocomplete="username" required>
<label for="local-password">Password</label><input type="password" id="local-password" name="password" autocomplete="current-password" required maxlength="1024">
<button class="secondary" type="submit">Sign in</button></form></section>"#
        )
    };
    page(
        if linking {
            "Connect LinkKeys"
        } else {
            "Sign in"
        },
        &format!(
            r#"{err}<section><h2>LinkKeys</h2><form method="post" action="/auth/login">{csrf_field}{link_field}{rt}
<label for="identity">Your LinkKeys identity</label>
<input type="text" id="identity" name="identity" placeholder="you@example.com" autocomplete="username" autofocus required>
<button type="submit">Continue with LinkKeys</button></form></section>{local}"#
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
    crate::http::blocking(|| {
        let target = destination(form.return_to.as_deref(), form.device_code.as_deref());
        let url = login_url(&target, form.link_local.unwrap_or(false));
        check_login_csrf(cookies, &form.csrf).map_err(|message| retry(&url, message))?;
        let req = vcsstar_types::BeginLoginRequest {
            ctx: vcsstar_types::Ctx {
                request_id: uuid::Uuid::now_v7().to_string(),
                deadline_ms: None,
                contract_version: None,
                contract_digest: None,
            },
            identity_input: form.identity.clone(),
            return_to: Some(target),
            device_user_code: None,
            link_local: form.link_local,
        };
        match login::begin(app, &session_ctx(app, cookies, &peer.0.remote), req) {
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
            Err(e) => Err(retry(&url, &e.message)),
        }
    })
}

fn check_login_csrf(cookies: &CookieJar<'_>, supplied: &str) -> Result<(), &'static str> {
    if !cookies
        .get(CSRF_COOKIE)
        .is_some_and(|c| !supplied.is_empty() && c.value() == supplied)
    {
        return Err("Reload the sign-in page");
    }
    Ok(())
}
#[derive(FromForm)]
struct LocalLoginForm {
    username: String,
    password: String,
    csrf: String,
    return_to: Option<String>,
    device_code: Option<String>,
}
#[post("/auth/local", data = "<form>")]
fn local_submit(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    form: Form<LocalLoginForm>,
) -> Result<Redirect, Redirect> {
    crate::http::blocking(|| {
        let target = destination(form.return_to.as_deref(), form.device_code.as_deref());
        let url = login_url(&target, false);
        check_login_csrf(cookies, &form.csrf).map_err(|message| retry(&url, message))?;
        let req = vcsstar_types::LocalLoginRequest {
            ctx: empty_ctx(),
            username: form.username.clone(),
            password: form.password.clone(),
            return_to: Some(target),
            device_user_code: None,
        };
        let session =
            vcsstar_core::services::local_login::sign_in(app, &anon_ctx(&peer.0.remote), req)
                .map_err(|e| retry(&url, &e.message))?;
        cookies.add(
            Cookie::build((SESSION_COOKIE, session.session_token))
                .http_only(true)
                .secure(app.config.public_url.starts_with("https://"))
                .same_site(SameSite::Lax)
                .path("/")
                .max_age(rocket::time::Duration::hours(login::SESSION_SLIDING_HOURS))
                .build(),
        );
        cookies.remove(Cookie::build(CSRF_COOKIE).path("/auth"));
        Ok(Redirect::to(
            session.return_to.unwrap_or_else(|| "/".into()),
        ))
    })
}

#[get("/auth/callback?<encrypted_token>&<error>")]
fn callback(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    encrypted_token: Option<String>,
    error: Option<String>,
) -> Result<Redirect, Redirect> {
    crate::http::blocking(|| {
        let attempt = cookies.get(ATTEMPT_COOKIE).map(|c| c.value());
        let retry_login = |message: &str| {
            let context = attempt.and_then(|a| login::retry_context(app, a).ok().flatten());
            let target = destination(context.as_ref().and_then(|c| c.return_to.as_deref()), None);
            retry(
                &login_url(&target, context.is_some_and(|c| c.link_local)),
                message,
            )
        };
        cookies.remove(Cookie::build(ATTEMPT_COOKIE).path("/auth"));
        if let Some(e) = error {
            return Err(retry_login(&format!("identity provider: {e}")));
        }
        let (Some(attempt), Some(token)) = (attempt, encrypted_token) else {
            return Err(retry_login("login attempt missing or expired"));
        };
        let req = vcsstar_types::CompleteLoginRequest {
            ctx: vcsstar_types::Ctx {
                request_id: uuid::Uuid::now_v7().to_string(),
                deadline_ms: None,
                contract_version: None,
                contract_digest: None,
            },
            attempt_token: attempt.to_string(),
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
                cookies.remove(Cookie::build(CSRF_COOKIE).path("/auth"));
                Ok(Redirect::to(destination(
                    session.return_to.as_deref(),
                    None,
                )))
            }
            Err(e) => Err(retry_login(&e.message)),
        }
    })
}

#[get("/auth/me")]
fn me(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
) -> Result<RawHtml<String>, Redirect> {
    crate::http::blocking(|| {
        if app.config.ui_dir.is_some() {
            return Err(Redirect::to("/me"));
        }
        let ctx = session_ctx(app, cookies, &peer.0.remote);
        let Some(p) = ctx.principal.as_ref() else {
            return Err(Redirect::to("/auth/login?return_to=/auth/me"));
        };
        Ok(page(
            "Signed in",
            &format!(
                "<p>You are <strong>{}@{}</strong> <span class=\"muted\">({})</span>.</p>\
<form method=\"post\" action=\"/auth/logout\"><button type=\"submit\">Sign out</button></form>",
                esc(&p.identity.handle),
                esc(&p.identity.lk_domain),
                esc(&p.key())
            ),
        ))
    })
}

#[post("/auth/logout")]
fn logout(app: &State<App>, cookies: &CookieJar<'_>, peer: HttpPeer) -> Redirect {
    crate::http::blocking(|| {
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
    })
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
    crate::http::blocking(|| {
        let ctx = session_ctx(app, cookies, &peer.0.remote);
        if ctx.principal.is_none() {
            let target = destination(Some("/auth/device"), code.as_deref());
            return Err(Redirect::to(login_url(&target, false)));
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
<button class=\"secondary\" type=\"submit\" name=\"decision\" value=\"deny\">Deny</button></form>\
<p class=\"muted\">Only approve a code you requested yourself a moment ago.</p>",
                esc(code.as_deref().unwrap_or(""))
            ),
        ))
    })
}

#[post("/auth/device", data = "<form>")]
fn device_submit(
    app: &State<App>,
    cookies: &CookieJar<'_>,
    peer: HttpPeer,
    form: Form<DeviceForm>,
) -> Redirect {
    crate::http::blocking(|| {
        let ctx = session_ctx(app, cookies, &peer.0.remote);
        if ctx.principal.is_none() {
            let target = destination(None, Some(&form.code));
            return Redirect::to(login_url(&target, false));
        }
        let req = vcsstar_types::DeviceApproveRequest {
            ctx: empty_ctx(),
            user_code: form.code.clone(),
            approve: form.decision == "approve",
        };
        match login::approve_device(app, &ctx, req) {
            Ok(_) => Redirect::to(rocket::uri!(device_result(form.decision == "approve"))),
            Err(e) => Redirect::to(format!(
                "/auth/device?code={}&msg={}",
                urlencode(&form.code),
                vcsstar_core::linkkeys::urlencode(&e.message)
            )),
        }
    })
}

#[get("/auth/device/result?<approved>")]
fn device_result(approved: bool) -> RawHtml<String> {
    page(
        if approved {
            "CLI login approved"
        } else {
            "CLI login denied"
        },
        "<p>You can close this page and return to your terminal.</p>",
    )
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
    crate::http::blocking(|| {
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
        match vcsstar_core::github_link::begin(app, &mut conn, p.identity.id, return_to.as_deref())
        {
            Ok(url) => Redirect::to(url),
            Err(e) => link_error(return_to.as_deref(), &e.message),
        }
    })
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
    crate::http::blocking(|| {
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
    })
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
        auth_style,
        login_form,
        login_submit,
        local_submit,
        callback,
        me,
        logout,
        device_form,
        device_submit,
        device_result,
        github_start,
        github_callback
    ]
}

#[allow(dead_code)]
fn _status_unused() -> Status {
    Status::Ok
}

#[cfg(test)]
mod local_form_tests {
    use super::*;
    use rocket::http::ContentType;
    use rocket::local::blocking::Client;

    #[test]
    fn browser_and_device_destinations() {
        let path = "/team/repo?view=files&ref=main#readme";
        assert_eq!(destination(Some(path), None), path);
        assert_eq!(destination(None, None), "/");
        assert_eq!(destination(Some("//example.com"), None), "/");
        assert_eq!(
            destination(None, Some("ABCD-EFGH")),
            "/auth/device?code=ABCD-EFGH"
        );
    }

    #[test]
    fn login_form_supplies_csrf_and_rejects_a_cross_site_post() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        std::env::set_var("VCSSTAR__DATABASE_URL", &url);
        let mut config = vcsstar_core::Config::from_env().unwrap();
        config.public_url = "http://localhost".into();
        let db = vcsstar_db::pool(&url, 2).unwrap();
        let app = App::new(config, db);
        let client = Client::tracked(rocket::build().manage(app).mount("/", routes())).unwrap();
        let device = client.get("/auth/device?code=ABCD-EFGH").dispatch();
        assert_eq!(
            device.headers().get_one("Location"),
            Some("/auth/login?return_to=%2Fauth%2Fdevice%3Fcode%3DABCD-EFGH")
        );
        let response = client
            .get("/auth/login?return_to=%2Fme%3Ftab%3Dkeys%23ssh")
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        let body = response.into_string().unwrap();
        assert!(body.contains("action=\"/auth/local\""));
        assert!(body.contains("autocomplete=\"current-password\""));
        assert!(body.contains("name=\"csrf\""));
        assert!(body.contains("value=\"/me?tab=keys#ssh\""));
        assert!(!body.contains("/auth/device"));
        assert!(client.cookies().get(CSRF_COOKIE).is_some());
        let response = client
            .post("/auth/local")
            .header(ContentType::Form)
            .body(
                "username=someone&password=anything&csrf=wrong&return_to=%2Fme%3Ftab%3Dkeys%23ssh",
            )
            .dispatch();
        assert_eq!(response.status(), Status::SeeOther);
        assert!(response
            .headers()
            .get_one("Location")
            .unwrap()
            .starts_with("/auth/login?return_to=%2Fme%3Ftab%3Dkeys%23ssh&error=Reload"));
        assert!(client.cookies().get(SESSION_COOKIE).is_none());
    }
}
