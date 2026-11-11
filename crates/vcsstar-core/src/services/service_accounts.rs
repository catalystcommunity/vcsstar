//! Service accounts: identities an organization owns that never sign in
//! through LinkKeys. They exist to hold API tokens for integrations that
//! do not act as a person (CI, bots, other systems). Their identity key is
//! `<user_id>@svc.<instance host>`, a domain no LinkKeys server serves,
//! so a login can never produce one. Grant them roles like any identity.

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_db::models::{ApiToken as ApiTokenRow, Identity, NewApiToken, NewIdentity};
use vcsstar_db::schema::{api_tokens, identities, service_accounts};
use vcsstar_db::DbConn;
use vcsstar_types::{
    ApiToken, CreateServiceAccountRequest, CreateServiceAccountTokenRequest, GetOrgRequest,
    ListServiceAccountsResponse, ListTokensResponse, Ok as WireOk,
    RevokeServiceAccountTokenRequest, ServiceAccount, ServiceAccountRef,
};

use crate::authz::{self, OrgRole};
use crate::outbox::{self, Emit};
use crate::principal::{identity_to_wire, CredentialKind, Principal, ReqCtx};
use crate::services::{auth, check_name};
use crate::tokens::{self, API_TOKEN_PREFIX};
use crate::{from_ms, to_ms, App, Code, Error};

/// The identity domain of this instance's service accounts.
pub fn service_domain(app: &App) -> String {
    let host = app
        .config
        .public_url
        .split("://")
        .nth(1)
        .unwrap_or(&app.config.public_url)
        .split('/')
        .next()
        .unwrap_or_default()
        .replace(':', "-")
        .to_ascii_lowercase();
    format!("svc.{host}")
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = service_accounts)]
struct Row {
    identity_id: uuid::Uuid,
    name: String,
    description: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

fn admin<'a>(
    app: &App,
    ctx: &'a ReqCtx,
    org: &str,
) -> Result<(DbConn, vcsstar_db::models::Org, &'a Principal), Error> {
    let p = ctx.require()?;
    p.require_scope("org:admin")?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    Ok((conn, org, p))
}

fn load(conn: &mut DbConn, org_id: uuid::Uuid, name: &str) -> Result<(Row, Identity), Error> {
    service_accounts::table
        .inner_join(identities::table.on(identities::id.eq(service_accounts::identity_id)))
        .filter(service_accounts::org_id.eq(org_id))
        .filter(service_accounts::name.eq(name))
        .select((Row::as_select(), Identity::as_select()))
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("service account"))
}

fn active_tokens(conn: &mut DbConn, identity_id: uuid::Uuid) -> Result<i64, Error> {
    Ok(api_tokens::table
        .filter(api_tokens::identity_id.eq(identity_id))
        .filter(api_tokens::revoked_at.is_null())
        .filter(
            api_tokens::expires_at
                .is_null()
                .or(api_tokens::expires_at.gt(Utc::now())),
        )
        .count()
        .get_result(conn)?)
}

fn to_wire(
    conn: &mut DbConn,
    org: &str,
    row: &Row,
    identity: &Identity,
) -> Result<ServiceAccount, Error> {
    Ok(ServiceAccount {
        identity: identity_to_wire(identity),
        org: org.to_string(),
        name: row.name.clone(),
        description: row.description.clone(),
        created_at: to_ms(row.created_at),
        disabled_at: identity.disabled_at.map(to_ms),
        active_tokens: active_tokens(conn, row.identity_id)?.max(0) as u64,
    })
}

fn note(
    conn: &mut DbConn,
    org: &vcsstar_db::models::Org,
    p: &Principal,
    kind: &str,
    what: String,
) -> Result<(), Error> {
    outbox::emit(
        conn,
        Emit {
            lane: outbox::lane_for_org(&org.name),
            kind,
            repo_id: None,
            org_id: Some(org.id),
            actor_id: Some(p.identity.id),
            actor: Some(p.to_wire()),
            repo: None,
            org: Some(org.name.clone()),
            subject: None,
            payload: vcsstar_types::EventPayload::Variant5(vcsstar_types::SettingsPayload {
                changed: vec![what],
            }),
        },
    )?;
    Ok(())
}

pub fn create(
    app: &App,
    ctx: &ReqCtx,
    req: CreateServiceAccountRequest,
) -> Result<ServiceAccount, Error> {
    let (mut conn, org, p) = admin(app, ctx, &req.org)?;
    check_name("service account", &req.name)?;
    let description = req
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    if description.is_some_and(|d| d.len() > 500) {
        return Err(Error::invalid("description must be at most 500 characters"));
    }
    let domain = service_domain(app);
    let (row, identity) = conn.transaction::<(Row, Identity), Error, _>(|conn| {
        let exists: Option<uuid::Uuid> = service_accounts::table
            .filter(service_accounts::org_id.eq(org.id))
            .filter(service_accounts::name.eq(&req.name))
            .select(service_accounts::identity_id)
            .first(conn)
            .optional()?;
        if exists.is_some() {
            return Err(Error::new(
                Code::AlreadyExists,
                format!(
                    "service account {} already exists in {}",
                    req.name, org.name
                ),
            ));
        }
        let identity: Identity = diesel::insert_into(identities::table)
            .values(&NewIdentity {
                id: vcsstar_db::new_id(),
                lk_user_id: vcsstar_db::new_id(),
                lk_domain: &domain,
                handle: &req.name,
                display_name: Some(&format!("{} (service account of {})", req.name, org.name)),
                email: None,
                email_verified: false,
            })
            .returning(Identity::as_returning())
            .get_result(conn)?;
        let row: Row = diesel::insert_into(service_accounts::table)
            .values((
                service_accounts::identity_id.eq(identity.id),
                service_accounts::org_id.eq(org.id),
                service_accounts::name.eq(&req.name),
                service_accounts::description.eq(description),
                service_accounts::created_by.eq(p.identity.id),
            ))
            .returning(Row::as_returning())
            .get_result(conn)?;
        note(
            conn,
            &org,
            p,
            "service_account_created",
            format!("service_account:{}", req.name),
        )?;
        Ok((row, identity))
    })?;
    to_wire(&mut conn, &org.name, &row, &identity)
}

pub fn list(
    app: &App,
    ctx: &ReqCtx,
    req: GetOrgRequest,
) -> Result<ListServiceAccountsResponse, Error> {
    let (mut conn, org, _p) = admin(app, ctx, &req.org)?;
    let rows: Vec<(Row, Identity)> = service_accounts::table
        .inner_join(identities::table.on(identities::id.eq(service_accounts::identity_id)))
        .filter(service_accounts::org_id.eq(org.id))
        .order(service_accounts::name.asc())
        .select((Row::as_select(), Identity::as_select()))
        .load(&mut conn)?;
    let mut accounts = Vec::with_capacity(rows.len());
    for (row, identity) in &rows {
        accounts.push(to_wire(&mut conn, &org.name, row, identity)?);
    }
    Ok(ListServiceAccountsResponse { accounts })
}

fn set_disabled(
    app: &App,
    ctx: &ReqCtx,
    req: ServiceAccountRef,
    disabled: bool,
) -> Result<WireOk, Error> {
    let (mut conn, org, p) = admin(app, ctx, &req.org)?;
    let (row, _identity) = load(&mut conn, org.id, &req.name)?;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(identities::table.find(row.identity_id))
            .set(identities::disabled_at.eq(disabled.then(Utc::now)))
            .execute(conn)?;
        note(
            conn,
            &org,
            p,
            if disabled {
                "service_account_disabled"
            } else {
                "service_account_enabled"
            },
            format!("service_account:{}", req.name),
        )?;
        Ok(())
    })?;
    Ok(WireOk { ok: true })
}

/// A disabled account's tokens are refused by the principal resolver
/// ("identity disabled") until `enable`; they are not revoked.
pub fn disable(app: &App, ctx: &ReqCtx, req: ServiceAccountRef) -> Result<WireOk, Error> {
    set_disabled(app, ctx, req, true)
}

pub fn enable(app: &App, ctx: &ReqCtx, req: ServiceAccountRef) -> Result<WireOk, Error> {
    set_disabled(app, ctx, req, false)
}

fn token_to_wire(t: &ApiTokenRow, secret: Option<String>) -> ApiToken {
    ApiToken {
        id: t.id.to_string(),
        name: t.name.clone(),
        scopes: t.scopes.iter().flatten().cloned().collect(),
        client_label: t.client_label.clone(),
        created_at: to_ms(t.created_at),
        expires_at: t.expires_at.map(to_ms),
        secret,
    }
}

/// Mint a token for the account. The same rules as `Auth/create-token`:
/// the caller must hold every scope the token gets, an API token caller
/// needs `tokens:manage`, and an act-as bearer may not mint.
pub fn create_token(
    app: &App,
    ctx: &ReqCtx,
    req: CreateServiceAccountTokenRequest,
) -> Result<ApiToken, Error> {
    let (mut conn, org, p) = admin(app, ctx, &req.org)?;
    if p.kind == CredentialKind::ActAs {
        return Err(Error::forbidden("an act-as bearer cannot mint tokens"));
    }
    if p.kind == CredentialKind::ApiToken {
        p.require_scope("tokens:manage")?;
    }
    auth::check_scopes(&req.scopes)?;
    for s in &req.scopes {
        if !p.has_scope(s) {
            return Err(Error::forbidden(format!("caller does not hold scope {s}")));
        }
    }
    if req.token_name.trim().is_empty() || req.token_name.len() > 100 {
        return Err(Error::invalid("token name must be 1-100 characters"));
    }
    let (row, identity) = load(&mut conn, org.id, &req.name)?;
    if identity.disabled_at.is_some() {
        return Err(Error::new(
            Code::PreconditionFailed,
            "the service account is disabled; enable it first",
        ));
    }
    let expires_at = req.expires_at.map(from_ms);
    if let Some(exp) = expires_at {
        if exp <= Utc::now() {
            return Err(Error::invalid("expires_at is in the past"));
        }
    }
    let secret = tokens::new_secret(API_TOKEN_PREFIX);
    let new = NewApiToken {
        id: vcsstar_db::new_id(),
        identity_id: row.identity_id,
        token_hash: tokens::hash(&secret),
        name: req.token_name.trim(),
        scopes: req.scopes.iter().cloned().map(Some).collect(),
        client_label: req.client_label.as_deref(),
        expires_at,
    };
    let inserted = conn.transaction::<ApiTokenRow, Error, _>(|conn| {
        let inserted: ApiTokenRow = diesel::insert_into(api_tokens::table)
            .values(&new)
            .returning(ApiTokenRow::as_returning())
            .get_result(conn)?;
        note(
            conn,
            &org,
            p,
            "service_account_token_created",
            format!("service_account:{}:token:{}", req.name, inserted.id),
        )?;
        Ok(inserted)
    })?;
    Ok(token_to_wire(&inserted, Some(secret)))
}

pub fn list_tokens(
    app: &App,
    ctx: &ReqCtx,
    req: ServiceAccountRef,
) -> Result<ListTokensResponse, Error> {
    let (mut conn, org, _p) = admin(app, ctx, &req.org)?;
    let (row, _identity) = load(&mut conn, org.id, &req.name)?;
    let rows: Vec<ApiTokenRow> = api_tokens::table
        .filter(api_tokens::identity_id.eq(row.identity_id))
        .filter(api_tokens::revoked_at.is_null())
        .order(api_tokens::created_at.desc())
        .select(ApiTokenRow::as_select())
        .load(&mut conn)?;
    Ok(ListTokensResponse {
        tokens: rows.iter().map(|t| token_to_wire(t, None)).collect(),
    })
}

pub fn revoke_token(
    app: &App,
    ctx: &ReqCtx,
    req: RevokeServiceAccountTokenRequest,
) -> Result<WireOk, Error> {
    let (mut conn, org, p) = admin(app, ctx, &req.org)?;
    let (row, _identity) = load(&mut conn, org.id, &req.name)?;
    let id = uuid::Uuid::parse_str(&req.token_id)
        .map_err(|_| Error::invalid("token_id must be a UUID"))?;
    conn.transaction::<(), Error, _>(|conn| {
        let n = diesel::update(
            api_tokens::table
                .filter(api_tokens::id.eq(id))
                .filter(api_tokens::identity_id.eq(row.identity_id))
                .filter(api_tokens::revoked_at.is_null()),
        )
        .set(api_tokens::revoked_at.eq(Utc::now()))
        .execute(conn)?;
        if n == 0 {
            return Err(Error::not_found("token"));
        }
        note(
            conn,
            &org,
            p,
            "service_account_token_revoked",
            format!("service_account:{}:token:{id}", req.name),
        )?;
        Ok(())
    })?;
    Ok(WireOk { ok: true })
}
