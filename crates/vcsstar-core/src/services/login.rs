//! LinkKeys browser login and the CLI device flow.
//!
//! Browser: `begin` stores a pending login bound to the browser by the hash
//! of an attempt token, asks the RP to sign the request, and returns the IDP
//! URL. `complete` claims the pending login, has the RP decrypt and verify
//! the assertion, checks domain, audience, nonce, and times itself, fetches
//! claims, upserts the identity, and issues a vcsstar session.
//!
//! Device: the CLI gets a device code and a short user code; the user
//! approves the user code in a logged-in browser; the CLI polls and receives
//! an API token once.

use chrono::{Duration, Utc};
use diesel::prelude::*;
use liblinkkeys::generated::types as lk;
use vcsstar_db::models::{
    ApiToken as ApiTokenRow, DeviceLogin, Identity, NewApiToken, NewDeviceLogin, NewIdentity,
    NewPendingLogin, NewSession, PendingLogin,
};
use vcsstar_db::schema::{api_tokens, device_logins, identities, pending_logins, sessions};
use vcsstar_db::DbConn;
use vcsstar_types::{
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
pub fn begin(app: &App, ctx: &ReqCtx, req: BeginLoginRequest) -> Result<BeginLoginResponse, Error> {
    let limits = &app.config.rate_limits;
    app.limiter
        .take(&peer_key("login", ctx), limits.login_per_minute, "login")?;
    let input = liblinkkeys::identity_input::parse_identity_input(&req.identity_input)
        .map_err(|e| Error::invalid(e.to_string()))?;
    app.limiter.take(
        &format!(
            "login:id:{}",
            req.identity_input.trim().to_ascii_lowercase()
        ),
        limits.login_per_minute,
        "login",
    )?;
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
        id: vcsstar_db::new_id(),
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
pub fn complete(app: &App, ctx: &ReqCtx, req: CompleteLoginRequest) -> Result<Session, Error> {
    app.limiter.take(
        &peer_key("complete", ctx),
        app.config.rate_limits.login_per_minute,
        "login",
    )?;
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
    // The RP verified the assertion; the claims it relayed are checked here
    // against the home domain's own signing keys (plan/01, trust policy).
    let home_keys = rp.resolve_domain_keys(&verified.assertion.domain)?;
    let claims = Claims::verified(
        &info,
        &verified.assertion.user_id,
        &verified.assertion.domain,
        &home_keys,
    )?;
    let handle = claims.handle.clone().ok_or_else(|| {
        Error::new(
            Code::Unauthenticated,
            "the identity provider released no handle claim; vcsstar requires it",
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

#[derive(Debug)]
struct Claims {
    handle: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    email_signed: bool,
}

/// What the home domain's signatures say about one claim.
enum Verdict {
    /// At least one valid signature by the home domain over this exact value.
    Verified,
    /// No home-domain signature at all; display data at best.
    Unsigned,
    /// Home-domain signatures are present but do not verify.
    Invalid(String),
}

/// Only the home domain's own signatures count: a handle or an email is
/// that domain's statement about its user. Third-party attestations are
/// ignored in v1.
fn home_domain_verdict(
    claim: &lk::Claim,
    domain: &str,
    keys: &[liblinkkeys::claims::DomainKeySet],
) -> Verdict {
    let mut own = claim.clone();
    own.signatures
        .retain(|s| s.domain.eq_ignore_ascii_case(domain));
    if own.signatures.is_empty() {
        return Verdict::Unsigned;
    }
    match liblinkkeys::claims::verify_claim(&own, domain, keys) {
        Ok(()) => Verdict::Verified,
        Err(e) => Verdict::Invalid(e.to_string()),
    }
}

impl Claims {
    /// Claims vcsstar trusts. `handle` needs a valid home-domain signature;
    /// a handle whose signatures fail is a login failure, not a fallback.
    /// `email` is stored as verified only with a valid signature; an
    /// unsigned email stays display data; one with bad signatures is
    /// dropped. `display_name` is display data.
    fn verified(
        info: &lk::UserInfo,
        user_id: &str,
        subject_domain: &str,
        home_keys: &[lk::DomainPublicKey],
    ) -> Result<Self, Error> {
        if !info.user_id.eq_ignore_ascii_case(user_id)
            || !info.domain.eq_ignore_ascii_case(subject_domain)
        {
            return Err(Error::new(
                Code::Unauthenticated,
                "userinfo identity does not match the assertion",
            ));
        }
        // The IDP signed its own domain string into every payload; use it.
        let domain = info.domain.as_str();
        let key_set = [liblinkkeys::claims::DomainKeySet {
            domain: domain.to_string(),
            keys: home_keys.to_vec(),
        }];
        let mut c = Claims {
            handle: None,
            display_name: None,
            email: None,
            email_signed: false,
        };
        let mut handle_rejected: Option<String> = None;
        for claim in &info.claims {
            if claim.revoked_at.is_some() || !claim.user_id.eq_ignore_ascii_case(user_id) {
                continue;
            }
            let value = String::from_utf8_lossy(&claim.claim_value)
                .trim()
                .to_string();
            if value.is_empty() {
                continue;
            }
            match claim.claim_type.as_str() {
                "handle" if c.handle.is_none() => {
                    match home_domain_verdict(claim, domain, &key_set) {
                        Verdict::Verified => c.handle = Some(value),
                        Verdict::Unsigned => {
                            handle_rejected
                                .get_or_insert_with(|| "no home-domain signature".to_string());
                        }
                        Verdict::Invalid(reason) => {
                            tracing::warn!(%domain, %reason, "handle claim signature rejected");
                            handle_rejected = Some(reason);
                        }
                    }
                }
                "display_name" if c.display_name.is_none() => c.display_name = Some(value),
                "email" if c.email.is_none() => {
                    match home_domain_verdict(claim, domain, &key_set) {
                        Verdict::Verified => {
                            c.email = Some(value);
                            c.email_signed = true;
                        }
                        Verdict::Unsigned => {
                            c.email = Some(value);
                            c.email_signed = false;
                        }
                        Verdict::Invalid(reason) => {
                            tracing::warn!(%domain, %reason, "email claim signature rejected; email dropped");
                        }
                    }
                }
                _ => {}
            }
        }
        if c.handle.is_none() {
            if let Some(reason) = handle_rejected {
                return Err(Error::new(
                    Code::Unauthenticated,
                    format!(
                        "the handle claim did not verify against the home domain's keys: {reason}"
                    ),
                ));
            }
        }
        if c.display_name.is_none() && !info.display_name.trim().is_empty() {
            c.display_name = Some(info.display_name.trim().to_string());
        }
        Ok(c)
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
                id: vcsstar_db::new_id(),
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
        id: vcsstar_db::new_id(),
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

pub fn begin_device(app: &App, ctx: &ReqCtx, req: BeginDeviceRequest) -> Result<DeviceCode, Error> {
    app.limiter.take(
        &peer_key("device", ctx),
        app.config.rate_limits.login_per_minute,
        "device login",
    )?;
    check_scopes(&req.scopes)?;
    if req.client_label.trim().is_empty() || req.client_label.len() > 100 {
        return Err(Error::invalid("client_label must be 1-100 characters"));
    }
    let device_code = tokens::new_secret(DEVICE_PREFIX);
    let code = user_code();
    let expires_at = Utc::now() + Duration::minutes(DEVICE_TTL_MINUTES);
    let row = NewDeviceLogin {
        id: vcsstar_db::new_id(),
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
    ctx: &ReqCtx,
    req: DevicePollRequest,
) -> Result<DevicePollResponse, Error> {
    let per_minute = app.config.rate_limits.device_poll_per_minute;
    app.limiter
        .take(&peer_key("poll", ctx), per_minute, "device poll")?;
    app.limiter.take(
        &crate::ratelimit::key_of_secret("poll", &req.device_code),
        per_minute,
        "device poll",
    )?;
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
                id: vcsstar_db::new_id(),
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
    // User codes are short; every attempt counts, approved or not.
    app.limiter.take(
        &format!("approve:{}", p.key()),
        app.config.rate_limits.device_approve_per_minute,
        "device approval",
    )?;
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

/// Bucket key for the peer address. An unknown address shares one bucket,
/// which only ever makes the limit stricter.
fn peer_key(what: &str, ctx: &ReqCtx) -> String {
    let remote = ctx.peer.remote.trim();
    format!(
        "{what}:ip:{}",
        if remote.is_empty() { "unknown" } else { remote }
    )
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
    use liblinkkeys::claims::{sign_claim, ClaimSigner, ClaimSpec};
    use liblinkkeys::crypto::{fingerprint, generate_keypair, SigningAlgorithm};

    const USER: &str = "01a11ea7-24d3-7612-8586-8e4645452613";

    struct Domain {
        name: &'static str,
        key_id: &'static str,
        public: lk::DomainPublicKey,
        private: Vec<u8>,
    }

    fn domain(name: &'static str, key_id: &'static str) -> Domain {
        let (public, private) = generate_keypair(SigningAlgorithm::Ed25519);
        let now = Utc::now();
        Domain {
            name,
            key_id,
            public: lk::DomainPublicKey {
                key_id: key_id.into(),
                fingerprint: fingerprint(&public),
                public_key: public,
                algorithm: "ed25519".into(),
                key_usage: "sign".into(),
                created_at: (now - Duration::days(1)).to_rfc3339(),
                expires_at: (now + Duration::days(1)).to_rfc3339(),
                revoked_at: None,
                signed_by_key_id: None,
                key_signature: None,
            },
            private,
        }
    }

    fn signed(d: &Domain, claim_type: &str, value: &str) -> lk::Claim {
        let attested = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        sign_claim(
            &ClaimSpec {
                claim_id: &format!("{claim_type}-1"),
                claim_type,
                claim_value: value.as_bytes(),
                user_id: USER,
                subject_domain: "home.test",
                expires_at: None,
                attested_at: &attested,
            },
            &[ClaimSigner {
                domain: d.name,
                key_id: d.key_id,
                algorithm: SigningAlgorithm::Ed25519,
                private_key_bytes: &d.private,
            }],
        )
        .expect("sign")
    }

    fn unsigned(claim_type: &str, value: &str) -> lk::Claim {
        let mut c = signed(&domain("x.test", "x"), claim_type, value);
        c.signatures.clear();
        c
    }

    fn info(claims: Vec<lk::Claim>) -> lk::UserInfo {
        lk::UserInfo {
            user_id: USER.into(),
            domain: "home.test".into(),
            display_name: "Alice".into(),
            claims,
        }
    }

    #[test]
    fn home_domain_signed_handle_and_email_verify() {
        let home = domain("home.test", "k1");
        let c = Claims::verified(
            &info(vec![
                signed(&home, "handle", "alice"),
                signed(&home, "email", "alice@home.test"),
            ]),
            USER,
            "home.test",
            std::slice::from_ref(&home.public),
        )
        .expect("verified");
        assert_eq!(c.handle.as_deref(), Some("alice"));
        assert_eq!(c.email.as_deref(), Some("alice@home.test"));
        assert!(c.email_signed);
        assert_eq!(c.display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn handle_signed_by_another_domain_is_refused() {
        let home = domain("home.test", "k1");
        let other = domain("other.test", "k9");
        let err = Claims::verified(
            &info(vec![signed(&other, "handle", "alice")]),
            USER,
            "home.test",
            std::slice::from_ref(&home.public),
        )
        .expect_err("refused");
        assert_eq!(err.code, Code::Unauthenticated);
    }

    #[test]
    fn tampered_handle_is_refused_and_unsigned_email_is_unverified() {
        let home = domain("home.test", "k1");
        let mut forged = signed(&home, "handle", "alice");
        forged.claim_value = b"mallory".to_vec();
        let err = Claims::verified(
            &info(vec![forged]),
            USER,
            "home.test",
            std::slice::from_ref(&home.public),
        )
        .expect_err("refused");
        assert!(err.message.contains("did not verify"), "{}", err.message);

        let c = Claims::verified(
            &info(vec![
                signed(&home, "handle", "alice"),
                unsigned("email", "alice@example.org"),
            ]),
            USER,
            "home.test",
            std::slice::from_ref(&home.public),
        )
        .expect("verified");
        assert_eq!(c.email.as_deref(), Some("alice@example.org"));
        assert!(!c.email_signed);
    }

    #[test]
    fn email_with_bad_home_signature_is_dropped() {
        let home = domain("home.test", "k1");
        let mut forged = signed(&home, "email", "alice@home.test");
        forged.claim_value = b"ceo@home.test".to_vec();
        let c = Claims::verified(
            &info(vec![signed(&home, "handle", "alice"), forged]),
            USER,
            "home.test",
            std::slice::from_ref(&home.public),
        )
        .expect("verified");
        assert!(c.email.is_none());
        assert!(!c.email_signed);
    }

    #[test]
    fn userinfo_for_another_identity_is_refused() {
        let home = domain("home.test", "k1");
        let err = Claims::verified(
            &info(vec![signed(&home, "handle", "alice")]),
            "01a11ea7-0000-7612-8586-8e4645452613",
            "home.test",
            std::slice::from_ref(&home.public),
        )
        .expect_err("refused");
        assert_eq!(err.code, Code::Unauthenticated);
    }

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
