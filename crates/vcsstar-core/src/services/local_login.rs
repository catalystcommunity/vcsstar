//! Local password login. A LinkKeys binding permanently disables this path.
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    principal::{identity_to_wire, ReqCtx},
    services::{instance, login, parse_identity_key},
    App, Code, Error,
};
use argon2::{
    password_hash::SaltString, Algorithm, Argon2, Params, PasswordHash, PasswordHasher,
    PasswordVerifier, Version,
};
use chrono::Utc;
use diesel::{
    prelude::*,
    sql_types::{Nullable, Text, Uuid as SqlUuid},
};
use vcsstar_db::{
    models::Identity,
    schema::{identities, sessions},
    DbConn,
};
use vcsstar_types::*;

const MEMORY_KIB: u32 = 65536;
const PASSES: u32 = 6;
static HASHES: AtomicUsize = AtomicUsize::new(0);
struct HashPermit;
impl HashPermit {
    fn acquire() -> Result<Self, Error> {
        let mut active = HASHES.load(Ordering::Relaxed);
        loop {
            if active >= 2 {
                return Err(Error::new(
                    Code::ResourceExhausted,
                    "password checks busy; try again",
                ));
            }
            match HASHES.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(Self),
                Err(current) => active = current,
            }
        }
    }
}
impl Drop for HashPermit {
    fn drop(&mut self) {
        HASHES.fetch_sub(1, Ordering::Release);
    }
}
fn algorithm() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(MEMORY_KIB, PASSES, 4, Some(32)).expect("fixed Argon2 parameters"),
    )
}
fn validate_password(password: &str) -> Result<(), Error> {
    if password.chars().count() < 15 || password.len() > 1024 {
        return Err(Error::invalid(
            "password must have at least 15 characters and at most 1024 bytes",
        ));
    }
    Ok(())
}
fn hash_password(password: &str) -> Result<String, Error> {
    validate_password(password)?;
    let _permit = HashPermit::acquire()?;
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    algorithm()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(Error::internal)
}
fn verify(password: &str, encoded: Option<&str>) -> Result<bool, Error> {
    if password.len() > 1024 {
        return Err(bad_login());
    }
    let _permit = HashPermit::acquire()?;
    if let Some(hash) = encoded.and_then(|s| PasswordHash::new(s).ok()) {
        return Ok(algorithm()
            .verify_password(password.as_bytes(), &hash)
            .is_ok());
    }
    // Keep unknown, disabled, and bound accounts on the expensive path too.
    let salt = SaltString::encode_b64(&[0u8; 16]).expect("fixed salt length");
    let _ = algorithm()
        .hash_password(password.as_bytes(), &salt)
        .map_err(Error::internal)?;
    Ok(false)
}
fn bad_login() -> Error {
    Error::new(Code::Unauthenticated, "invalid username or password")
}
fn rate(app: &App, ctx: &ReqCtx, name: &str) -> Result<(), Error> {
    let limit = app.config.rate_limits.login_per_minute;
    app.limiter
        .take(&format!("local:peer:{}", ctx.peer.remote), limit, "login")?;
    app.limiter
        .take(&format!("local:user:{name}"), limit, "login")
}
#[derive(QueryableByName)]
struct PasswordRow {
    #[diesel(sql_type = SqlUuid)]
    identity_id: uuid::Uuid,
    #[diesel(sql_type = Nullable<Text>)]
    password_hash: Option<String>,
}
fn local_row(conn: &mut DbConn, id: uuid::Uuid) -> Result<PasswordRow, Error> {
    diesel::sql_query("SELECT l.identity_id,l.password_hash FROM local_accounts l JOIN identities i ON i.id=l.identity_id WHERE i.id=$1 AND NOT i.linkkeys_bound AND i.disabled_at IS NULL FOR UPDATE OF i,l")
        .bind::<SqlUuid,_>(id).get_result(conn).optional()?.ok_or_else(|| Error::forbidden("local password login is not available for this account"))
}
fn emit(
    conn: &mut DbConn,
    actor: Option<&crate::principal::Principal>,
    id: uuid::Uuid,
    kind: &str,
) -> Result<(), Error> {
    crate::outbox::emit(
        conn,
        crate::outbox::Emit {
            lane: "@instance".into(),
            kind,
            repo_id: None,
            org_id: None,
            actor_id: actor.map(|p| p.identity.id),
            actor: actor.map(|p| p.to_wire()),
            repo: None,
            org: None,
            subject: Some(id.to_string()),
            payload: EventPayload::Variant5(SettingsPayload {
                changed: vec!["local_account".into()],
            }),
        },
    )?;
    Ok(())
}
pub fn account(app: &App, ctx: &ReqCtx, _: Empty) -> Result<LocalAccount, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    #[derive(QueryableByName)]
    struct Name {
        #[diesel(sql_type=Text)]
        username: String,
    }
    let name: Option<Name> =
        diesel::sql_query("SELECT username FROM local_accounts WHERE identity_id=$1")
            .bind::<SqlUuid, _>(p.identity.id)
            .get_result(&mut conn)
            .optional()?;
    Ok(LocalAccount {
        identity: p.to_wire(),
        password_login: !p.identity.linkkeys_bound && name.is_some(),
        username: name.map(|n| n.username),
    })
}
pub fn create(
    app: &App,
    ctx: &ReqCtx,
    req: CreateLocalAccountRequest,
) -> Result<LocalAccount, Error> {
    let p = instance::require_admin(app, ctx)?;
    let username = req.username.trim().to_ascii_lowercase();
    super::check_name("username", &username)?;
    let hash = hash_password(&req.password)?;
    let mut conn = app.conn()?;
    conn.transaction::<_, Error, _>(|conn| {
        let id = vcsstar_db::new_id();
        let identity: Identity = diesel::insert_into(identities::table)
            .values((
                identities::id.eq(id),
                identities::lk_user_id.eq(id),
                identities::lk_domain.eq("local.invalid"),
                identities::handle.eq(&username),
                identities::display_name.eq(req.display_name.as_deref()),
                identities::linkkeys_bound.eq(false),
            ))
            .returning(Identity::as_returning())
            .get_result(conn)?;
        diesel::sql_query(
            "INSERT INTO local_accounts(identity_id,username,password_hash) VALUES($1,$2,$3)",
        )
        .bind::<SqlUuid, _>(id)
        .bind::<Text, _>(&username)
        .bind::<Text, _>(&hash)
        .execute(conn)?;
        emit(conn, Some(p), id, "local_account_created")?;
        Ok(LocalAccount {
            identity: identity_to_wire(&identity),
            username: Some(username.clone()),
            password_login: true,
        })
    })
}
pub fn reset(
    app: &App,
    ctx: &ReqCtx,
    req: ResetLocalPasswordRequest,
) -> Result<vcsstar_types::Ok, Error> {
    let p = instance::require_admin(app, ctx)?;
    let (id, domain) = parse_identity_key(&req.identity)?;
    let hash = hash_password(&req.password)?;
    let mut conn = app.conn()?;
    conn.transaction::<_, Error, _>(|conn| {
        let id: uuid::Uuid = identities::table
            .filter(identities::lk_user_id.eq(id))
            .filter(identities::lk_domain.eq(domain))
            .select(identities::id)
            .first(conn)?;
        local_row(conn, id)?;
        replace_password(conn, id, &hash)?;
        emit(conn, Some(p), id, "local_password_reset")?;
        Ok(vcsstar_types::Ok { ok: true })
    })
}
fn replace_password(conn: &mut DbConn, id: uuid::Uuid, hash: &str) -> Result<(), Error> {
    diesel::sql_query(
        "UPDATE local_accounts SET password_hash=$2,updated_at=now() WHERE identity_id=$1",
    )
    .bind::<SqlUuid, _>(id)
    .bind::<Text, _>(hash)
    .execute(conn)?;
    diesel::delete(sessions::table.filter(sessions::identity_id.eq(id))).execute(conn)?;
    // A password reset also closes tokens obtained through the device flow.
    diesel::sql_query(
        "UPDATE api_tokens SET revoked_at=now() WHERE identity_id=$1 AND revoked_at IS NULL",
    )
    .bind::<SqlUuid, _>(id)
    .execute(conn)?;
    Ok(())
}
pub fn change(
    app: &App,
    ctx: &ReqCtx,
    req: ChangeLocalPasswordRequest,
) -> Result<vcsstar_types::Ok, Error> {
    let p = ctx.require()?;
    p.require_scope("auth:write")?;
    rate(app, ctx, &p.key())?;
    let mut conn = app.conn()?;
    conn.transaction::<_, Error, _>(|conn| {
        let row = local_row(conn, p.identity.id)?;
        if !verify(&req.current_password, row.password_hash.as_deref())? {
            return Err(bad_login());
        }
        let hash = hash_password(&req.new_password)?;
        replace_password(conn, p.identity.id, &hash)?;
        emit(conn, Some(p), p.identity.id, "local_password_changed")?;
        Ok(vcsstar_types::Ok { ok: true })
    })
}
pub fn sign_in(app: &App, ctx: &ReqCtx, req: LocalLoginRequest) -> Result<Session, Error> {
    let username = req.username.trim().to_ascii_lowercase();
    if username.len() > 100 {
        return Err(bad_login());
    }
    rate(app, ctx, &username)?;
    login::check_return_to(req.return_to.as_deref())?;
    let mut conn = app.conn()?;
    conn.transaction::<_,Error,_>(|conn| {
        let row: Option<PasswordRow> = diesel::sql_query("SELECT l.identity_id,l.password_hash FROM local_accounts l JOIN identities i ON i.id=l.identity_id WHERE l.username=$1 AND NOT i.linkkeys_bound AND i.disabled_at IS NULL FOR UPDATE OF i,l")
            .bind::<Text,_>(&username).get_result(conn).optional()?;
        if !verify(&req.password,row.as_ref().and_then(|r|r.password_hash.as_deref()))? { return Err(bad_login()); }
        let id=row.expect("verified stored password").identity_id;
        let identity: Identity=diesel::update(identities::table.find(id)).set(identities::last_login.eq(Utc::now())).returning(Identity::as_returning()).get_result(conn)?;
        if let Some(code)=req.device_user_code.as_deref() {
            let device=login::find_device_by_user_code(conn,code)?;
            login::approve_device_by_id(conn,device.id,&identity)?;
        }
        let (session_token,expires)=login::create_session(conn,id,None)?;
        emit(conn,None,id,"local_login")?;
        Ok(Session {session_token,identity:identity_to_wire(&identity),expires_at:crate::to_ms(expires),return_to:req.return_to.clone()})
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn salted_argon2id_and_hash_limit() {
        let password = "a sufficiently long test password";
        let first = hash_password(password).unwrap();
        let second = hash_password(password).unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("$argon2id$v=19$m=65536,t=6,p=4$"));
        assert!(verify(password, Some(&first)).unwrap());
        assert!(!verify("incorrect", Some(&first)).unwrap());
        assert!(hash_password("short").is_err());

        let first = HashPermit::acquire().unwrap();
        let second = HashPermit::acquire().unwrap();
        assert!(matches!(
            HashPermit::acquire(),
            Err(Error {
                code: Code::ResourceExhausted,
                ..
            })
        ));
        drop(first);
        let replacement = HashPermit::acquire().unwrap();
        drop((second, replacement));
        assert_eq!(HASHES.load(Ordering::Relaxed), 0);
    }
}
