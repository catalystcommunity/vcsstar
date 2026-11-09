//! LinkKeys browser login and the CLI device flow.
//!
//! Browser: `begin` stores a pending login bound to the browser by the hash
//! of an attempt token, asks the RP to sign the request, and returns the IDP
//! URL. `complete` claims the pending login, has the RP decrypt and verify
//! the assertion, checks domain, audience, nonce, and times itself, fetches
//! claims, upserts the identity, and issues a vscstar session.
//!
//! Device: the CLI gets a device code and a short user code; the user
//! approves the user code in a logged-in browser; the CLI polls and receives
//! an API token once.

use chrono::{Duration, Utc};
use diesel::prelude::*;
use liblinkkeys::generated::types as lk;
use vscstar_db::models::{
    ApiToken as ApiTokenRow, DeviceLogin, Identity, NewApiToken, NewDeviceLogin, NewIdentity,
    NewPendingLogin, NewSession, PendingLogin,
};
use vscstar_db::schema::{api_tokens, device_logins, identities, pending_logins, sessions};
use vscstar_db::DbConn;
use vscstar_types::{
    ApiToken, BeginDeviceRequest, BeginLoginRequest, BeginLoginResponse, CompleteLoginRequest,
    DeviceApproveRequest, DeviceCode, DevicePollRequest, DevicePollResponse, Ok as WireOk, Session,
};

use crate::linkkeys::{self, check_assertion};
use crate::principal::{identity_to_wire, ReqCtx};
use crate::services::auth::check_scopes;
use crate::tokens::{self, API_TOKEN_PREFIX, SESSION_PREFIX};
use crate::{to_ms, App, Code, Error};

pub const LOGIN_TTL_MINUTES: i64 = 10;
pub const DEVICE_TTL_MINUTES: i64 = 10;
pub const SESSION_SLIDING_HOURS: i64 = 12;
pub const ATTEMPT_PREFIX: &str = "vsa_";
pub const DEVICE_PREFIX: &str = "vsd_";

pub fn callback_url(app: &App) -> String {
    format!(
        "{}/auth/callback",
        app.config.public_url.trim_end_matches('/')
    )
}

/// Step 1. The browser (or the UI through the API) submits the identity.
pub fn begin(
    app: &App,
    _ctx: &ReqCtx,
    req: BeginLoginRequest,
) -> Result<BeginLoginResponse, Error> {
    let input = liblinkkeys::identity_input::parse_identity_input(&req.identity_input)
        .map_err(|e| Error::invalid(e.to_string()))?;
    let rp = app.rp()?;
    let nonce = hex::encode(rand_bytes(16));
    let signed = rp.sign_request(&callback_url(app), &nonce)?;
    let base = linkkeys::browser_base(&input.domain);
    let redirect_url =
        linkkeys::authorize_url(&base, &signed, input.username.as_deref().unwrap_or(""));

    let attempt_token = tokens::new_secret(ATTEMPT_PREFIX);
    let expires_at = Utc::now() + Duration::minutes(LOGIN_TTL_MINUTES);
    let device_login_id = match req.device_user_code.as_deref() {
        Some(code) => Some(find_device_by_user_code(&mut app.conn()?, code)?.id),
        None => None,
    };
    if let Some(rt) = req.return_to.as_deref() {
        if !rt.starts_with('/') || rt.starts_with("//") {
            return Err(Error::invalid("return_to must be a local path"));
        }
    }
    let row = NewPendingLogin {
        id: vscstar_db::new_id(),
        attempt_hash: tokens::hash(&attempt_token),
        nonce: &nonce,
        identity_input: &req.identity_input,
        idp_domain: &input.domain,
        return_to: req.return_to.as_deref(),
        device_login_id,
        expires_at,
    };
    let mut conn = app.conn()?;
    diesel::insert_into(pending_logins::table)
        .values(&row)
        .execute(&mut conn)?;
    Ok(BeginLoginResponse {
        redirect_url,
        attempt_token,
        expires_at: to_ms(expires_at),
    })
}

/// Step 2. The IDP redirected back with `encrypted_token`; the attempt token
/// came back in the browser's cookie.
pub fn complete(app: &App, _ctx: &ReqCtx, req: CompleteLoginRequest) -> Result<Session, Error> {
    let mut conn = app.conn()?;
    let pending = claim_pending(&mut conn, &req.attempt_token)?;
    let rp = app.rp()?;
    let rp_domain = app
        .config
        .linkkeys
        .rp_domain
        .clone()
        .unwrap_or_else(|| app.config.instance_domain.clone());

    let signed_assertion = rp.decrypt_token(&req.encrypted_token)?;
    let verified = rp.verify_assertion(&signed_assertion, &pending.idp_domain)?;
    if !verified.verified {
        return Err(Error::new(
            Code::Unauthenticated,
            "the identity provider's assertion did not verify",
        ));
    }
    check_assertion(
        &verified.assertion,
        &pending.idp_domain,
        &rp_domain,
        &pending.nonce,
    )?;

    let api_base = linkkeys::browser_base(&pending.idp_domain);
    let info = rp.userinfo_fetch(&signed_assertion, &api_base, &pending.idp_domain)?;
    let claims = Claims::from_userinfo(&info);
    let handle = claims.handle.clone().ok_or_else(|| {
        Error::new(
            Code::Unauthenticated,
            "the identity provider released no handle claim; vscstar requires it",
        )
    })?;

    let user_id = uuid::Uuid::parse_str(&verified.assertion.user_id)
        .map_err(|_| Error::new(Code::Unauthenticated, "assertion user_id is not a UUID"))?;
    let domain = verified.assertion.domain.to_ascii_lowercase();
    let identity = upsert_identity(&mut conn, user_id, &domain, &handle, &claims)?;
    if identity.disabled_at.is_some() {
        return Err(Error::forbidden("identity disabled"));
    }

    if let Some(device_id) = pending.device_login_id {
        approve_device_by_id(&mut conn, device_id, &identity)?;
    }

    let (session_token, expires_at) = create_session(&mut conn, identity.id, None)?;
    Ok(Session {
        session_token,
        identity: identity_to_wire(&identity),
        expires_at: to_ms(expires_at),
        return_to: pending.return_to,
    })
}

struct Claims {
    handle: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    email_signed: bool,
}

impl Claims {
    fn from_userinfo(info: &lk::UserInfo) -> Self {
        let mut c = Claims {
            handle: None,
            display_name: None,
            email: None,
            email_signed: false,
        };
        for claim in &info.claims {
            if claim.revoked_at.is_some() {
                continue;
            }
            let value = String::from_utf8_lossy(&claim.claim_value)
                .trim()
                .to_string();
            if value.is_empty() {
                continue;
            }
            match claim.claim_type.as_str() {
                "handle" if c.handle.is_none() => c.handle = Some(value),
                "display_name" if c.display_name.is_none() => c.display_name = Some(value),
                "email" if c.email.is_none() => {
                    c.email_signed = !claim.signatures.is_empty();
                    c.email = Some(value);
                }
                _ => {}
            }
        }
        if c.display_name.is_none() && !info.display_name.trim().is_empty() {
            c.display_name = Some(info.display_name.trim().to_string());
        }
        c
    }
}

fn upsert_identity(
    conn: &mut DbConn,
    user_id: uuid::Uuid,
    domain: &str,
    handle: &str,
    claims: &Claims,
) -> Result<Identity, Error> {
    let existing: Option<Identity> = identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq(domain))
        .select(Identity::as_select())
        .first(conn)
        .optional()?;
    let now = Utc::now();
    match existing {
        Some(i) => {
            let updated: Identity = diesel::update(identities::table.find(i.id))
                .set((
                    identities::handle.eq(handle),
                    identities::display_name.eq(claims.display_name.as_deref()),
                    identities::email.eq(claims.email.as_deref()),
                    identities::email_verified.eq(claims.email.is_some() && claims.email_signed),
                    identities::last_login.eq(now),
                ))
                .returning(Identity::as_returning())
                .get_result(conn)?;
            Ok(updated)
        }
        None => {
            let row = NewIdentity {
                id: vscstar_db::new_id(),
                lk_user_id: user_id,
                lk_domain: domain,
                handle,
                display_name: claims.display_name.as_deref(),
                email: claims.email.as_deref(),
                email_verified: claims.email.is_some() && claims.email_signed,
            };
            let inserted: Identity = diesel::insert_into(identities::table)
                .values(&row)
                .returning(Identity::as_returning())
                .get_result(conn)?;
            diesel::update(identities::table.find(inserted.id))
                .set(identities::last_login.eq(now))
                .execute(conn)?;
            Ok(inserted)
        }
    }
}

fn claim_pending(conn: &mut DbConn, attempt_token: &str) -> Result<PendingLogin, Error> {
    let digest = tokens::hash(attempt_token.trim());
    let claimed: Option<PendingLogin> = diesel::update(
        pending_logins::table
            .filter(pending_logins::attempt_hash.eq(&digest))
            .filter(pending_logins::claimed_at.is_null())
            .filter(pending_logins::expires_at.gt(Utc::now())),
    )
    .set(pending_logins::claimed_at.eq(Utc::now()))
    .returning(PendingLogin::as_returning())
    .get_result(conn)
    .optional()?;
    claimed.ok_or_else(|| {
        Error::new(
            Code::Unauthenticated,
            "login attempt is unknown, used, or expired; start again",
        )
    })
}

pub fn create_session(
    conn: &mut DbConn,
    identity_id: uuid::Uuid,
    user_agent: Option<&str>,
) -> Result<(String, chrono::DateTime<Utc>), Error> {
    let token = tokens::new_secret(SESSION_PREFIX);
    let expires_at = Utc::now() + Duration::hours(SESSION_SLIDING_HOURS);
    let row = NewSession {
        id: vscstar_db::new_id(),
        identity_id,
        token_hash: tokens::hash(&token),
        expires_at,
        user_agent,
    };
    diesel::insert_into(sessions::table)
        .values(&row)
        .execute(conn)?;
    Ok((token, expires_at))
}

// ---- device flow -----------------------------------------------------

fn rand_bytes(n: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

/// Eight characters from an alphabet without look-alikes, shown as XXXX-XXXX.
fn user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
    let bytes = rand_bytes(8);
    let s: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &s[..4], &s[4..])
}

fn normalize_user_code(s: &str) -> String {
    s.trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn find_device_by_user_code(conn: &mut DbConn, code: &str) -> Result<DeviceLogin, Error> {
    let digest = tokens::hash(&normalize_user_code(code));
    device_logins::table
        .filter(device_logins::user_code_hash.eq(&digest))
        .filter(device_logins::state.eq("pending"))
        .filter(device_logins::expires_at.gt(Utc::now()))
        .select(DeviceLogin::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("device code (expired or already used)"))
}

pub fn begin_device(
    app: &App,
    _ctx: &ReqCtx,
    req: BeginDeviceRequest,
) -> Result<DeviceCode, Error> {
    check_scopes(&req.scopes)?;
    if req.client_label.trim().is_empty() || req.client_label.len() > 100 {
        return Err(Error::invalid("client_label must be 1-100 characters"));
    }
    let device_code = tokens::new_secret(DEVICE_PREFIX);
    let code = user_code();
    let expires_at = Utc::now() + Duration::minutes(DEVICE_TTL_MINUTES);
    let row = NewDeviceLogin {
        id: vscstar_db::new_id(),
        code_hash: tokens::hash(&device_code),
        user_code_hash: tokens::hash(&normalize_user_code(&code)),
        client_label: req.client_label.trim(),
        scopes: req.scopes.iter().cloned().map(Some).collect(),
        state: "pending",
        expires_at,
    };
    let mut conn = app.conn()?;
    diesel::insert_into(device_logins::table)
        .values(&row)
        .execute(&mut conn)?;
    Ok(DeviceCode {
        device_code,
        user_code: code.clone(),
        verification_url: format!(
            "{}/auth/device?code={}",
            app.config.public_url.trim_end_matches('/'),
            code
        ),
        expires_at: to_ms(expires_at),
        poll_interval_ms: 3000,
    })
}

pub fn poll_device(
    app: &App,
    _ctx: &ReqCtx,
    req: DevicePollRequest,
) -> Result<DevicePollResponse, Error> {
    let mut conn = app.conn()?;
    let digest = tokens::hash(req.device_code.trim());
    let row: Option<DeviceLogin> = device_logins::table
        .filter(device_logins::code_hash.eq(&digest))
        .select(DeviceLogin::as_select())
        .first(&mut conn)
        .optional()?;
    let row = row.ok_or_else(|| Error::new(Code::Unauthenticated, "unknown device code"))?;
    if row.expires_at <= Utc::now() && row.state == "pending" {
        return Err(Error::new(
            Code::TokenExpired,
            "device code expired; start again",
        ));
    }
    match row.state.as_str() {
        "pending" => Ok(DevicePollResponse {
            pending: true,
            token: None,
        }),
        "denied" => Err(Error::forbidden("the user denied this login")),
        "consumed" | "expired" => Err(Error::new(
            Code::Unauthenticated,
            "device code already used",
        )),
        "approved" => {
            let identity_id = row
                .identity_id
                .ok_or_else(|| Error::internal("approved device login without identity"))?;
            let scopes: Vec<String> = row.scopes.iter().flatten().cloned().collect();
            let secret = tokens::new_secret(API_TOKEN_PREFIX);
            let token = NewApiToken {
                id: vscstar_db::new_id(),
                identity_id,
                token_hash: tokens::hash(&secret),
                name: &row.client_label,
                scopes: scopes.iter().cloned().map(Some).collect(),
                client_label: Some(&row.client_label),
                expires_at: Some(Utc::now() + Duration::days(90)),
            };
            let inserted: ApiTokenRow = conn.transaction::<ApiTokenRow, Error, _>(|conn| {
                let inserted: ApiTokenRow = diesel::insert_into(api_tokens::table)
                    .values(&token)
                    .returning(ApiTokenRow::as_returning())
                    .get_result(conn)?;
                let n = diesel::update(
                    device_logins::table
                        .find(row.id)
                        .filter(device_logins::state.eq("approved")),
                )
                .set((
                    device_logins::state.eq("consumed"),
                    device_logins::api_token_id.eq(inserted.id),
                ))
                .execute(conn)?;
                if n == 0 {
                    return Err(Error::new(
                        Code::Conflict,
                        "device code consumed concurrently",
                    ));
                }
                Ok(inserted)
            })?;
            Ok(DevicePollResponse {
                pending: false,
                token: Some(ApiToken {
                    id: inserted.id.to_string(),
                    name: inserted.name.clone(),
                    scopes,
                    client_label: inserted.client_label.clone(),
                    created_at: to_ms(inserted.created_at),
                    expires_at: inserted.expires_at.map(to_ms),
                    secret: Some(secret),
                }),
            })
        }
        other => Err(Error::internal(format!(
            "device login in unknown state {other}"
        ))),
    }
}

fn approve_device_by_id(
    conn: &mut DbConn,
    id: uuid::Uuid,
    identity: &Identity,
) -> Result<(), Error> {
    let n = diesel::update(
        device_logins::table
            .find(id)
            .filter(device_logins::state.eq("pending"))
            .filter(device_logins::expires_at.gt(Utc::now())),
    )
    .set((
        device_logins::state.eq("approved"),
        device_logins::identity_id.eq(identity.id),
    ))
    .execute(conn)?;
    if n == 0 {
        return Err(Error::not_found("device code (expired or already used)"));
    }
    Ok(())
}

/// Browser side: a logged-in user approves or denies a user code.
pub fn approve_device(app: &App, ctx: &ReqCtx, req: DeviceApproveRequest) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    if p.kind != crate::principal::CredentialKind::Session {
        return Err(Error::forbidden("device approval needs a browser session"));
    }
    let mut conn = app.conn()?;
    let row = find_device_by_user_code(&mut conn, &req.user_code)?;
    if req.approve {
        approve_device_by_id(&mut conn, row.id, &p.identity)?;
    } else {
        diesel::update(device_logins::table.find(row.id))
            .set(device_logins::state.eq("denied"))
            .execute(&mut conn)?;
    }
    Ok(WireOk { ok: true })
}

/// Maintenance: drop expired pending logins and device codes.
pub fn purge_expired(conn: &mut DbConn) -> Result<usize, Error> {
    let a = diesel::delete(
        pending_logins::table
            .filter(pending_logins::expires_at.lt(Utc::now() - Duration::hours(1))),
    )
    .execute(conn)?;
    let b = diesel::delete(
        device_logins::table.filter(device_logins::expires_at.lt(Utc::now() - Duration::hours(24))),
    )
    .execute(conn)?;
    Ok(a + b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_have_no_lookalikes() {
        for _ in 0..50 {
            let c = user_code();
            assert_eq!(c.len(), 9);
            assert!(!c.contains(['0', 'O', '1', 'I', 'A', 'E', 'U', 'Y']), "{c}");
        }
        assert_eq!(normalize_user_code(" bcdf-ghjk "), "BCDFGHJK");
    }
}
