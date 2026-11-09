//! Tokens, sessions, and identity introspection. The LinkKeys login flows
//! live in `linkkeys.rs` (phase 0, second half) and call into here.

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_db::models::{ApiToken as ApiTokenRow, NewApiToken};
use vcsstar_db::schema::{api_tokens, sessions};
use vcsstar_types::{
    ApiToken, CreateTokenRequest, Empty, ListTokensResponse, Ok as WireOk, RevokeTokenRequest,
    WhoAmI,
};

use crate::principal::{CredentialKind, ReqCtx};
use crate::tokens::{self, API_TOKEN_PREFIX};
use crate::{from_ms, to_ms, App, Error};

pub const KNOWN_SCOPES: &[&str] = &[
    "admin",
    "org:read",
    "org:admin",
    "repo:read",
    "repo:write",
    "repo:admin",
    "pr:read",
    "pr:write",
    "pr:review",
    "statuses:write",
    "hooks:manage",
    "mirror:manage",
    "tokens:manage",
];

pub fn check_scopes(scopes: &[String]) -> Result<(), Error> {
    if scopes.is_empty() {
        return Err(Error::invalid("at least one scope is required"));
    }
    for s in scopes {
        if !KNOWN_SCOPES.contains(&s.as_str()) {
            return Err(Error::invalid(format!("unknown scope {s}")));
        }
    }
    Ok(())
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

pub fn whoami(_app: &App, ctx: &ReqCtx, _req: Empty) -> Result<WhoAmI, Error> {
    let p = ctx.require()?;
    Ok(WhoAmI {
        identity: p.to_wire(),
        scopes: p.scopes.clone(),
        credential_kind: p.kind.as_str().to_string(),
        acting_via: p.acting_via.clone(),
        expires_at: p.expires_at.map(to_ms),
    })
}

/// Create an API token for the caller. A token can only carry scopes the
/// caller holds, so a scoped token never escalates.
pub fn create_token(app: &App, ctx: &ReqCtx, req: CreateTokenRequest) -> Result<ApiToken, Error> {
    let p = ctx.require()?;
    if p.kind == CredentialKind::ActAs {
        return Err(Error::forbidden("an act-as bearer cannot mint tokens"));
    }
    if p.kind == CredentialKind::ApiToken {
        p.require_scope("tokens:manage")?;
    }
    check_scopes(&req.scopes)?;
    for s in &req.scopes {
        if !p.has_scope(s) {
            return Err(Error::forbidden(format!("caller does not hold scope {s}")));
        }
    }
    if req.name.trim().is_empty() || req.name.len() > 100 {
        return Err(Error::invalid("token name must be 1-100 characters"));
    }
    let secret = tokens::new_secret(API_TOKEN_PREFIX);
    let expires_at = req.expires_at.map(from_ms);
    if let Some(exp) = expires_at {
        if exp <= Utc::now() {
            return Err(Error::invalid("expires_at is in the past"));
        }
    }
    let row = NewApiToken {
        id: vcsstar_db::new_id(),
        identity_id: p.identity.id,
        token_hash: tokens::hash(&secret),
        name: req.name.trim(),
        scopes: req.scopes.iter().cloned().map(Some).collect(),
        client_label: req.client_label.as_deref(),
        expires_at,
    };
    let mut conn = app.conn()?;
    let inserted: ApiTokenRow = diesel::insert_into(api_tokens::table)
        .values(&row)
        .returning(ApiTokenRow::as_returning())
        .get_result(&mut conn)?;
    Ok(token_to_wire(&inserted, Some(secret)))
}

pub fn revoke_token(app: &App, ctx: &ReqCtx, req: RevokeTokenRequest) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    let id = uuid::Uuid::parse_str(&req.token_id)
        .map_err(|_| Error::invalid("token_id must be a UUID"))?;
    let mut conn = app.conn()?;
    let n = diesel::update(
        api_tokens::table
            .filter(api_tokens::id.eq(id))
            .filter(api_tokens::identity_id.eq(p.identity.id))
            .filter(api_tokens::revoked_at.is_null()),
    )
    .set(api_tokens::revoked_at.eq(Utc::now()))
    .execute(&mut conn)?;
    if n == 0 {
        return Err(Error::not_found("token"));
    }
    Ok(WireOk { ok: true })
}

pub fn list_tokens(app: &App, ctx: &ReqCtx, _req: Empty) -> Result<ListTokensResponse, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    let rows: Vec<ApiTokenRow> = api_tokens::table
        .filter(api_tokens::identity_id.eq(p.identity.id))
        .filter(api_tokens::revoked_at.is_null())
        .order(api_tokens::created_at.desc())
        .select(ApiTokenRow::as_select())
        .load(&mut conn)?;
    Ok(ListTokensResponse {
        tokens: rows.iter().map(|t| token_to_wire(t, None)).collect(),
    })
}

/// Revoke the credential used for this call.
pub fn logout(app: &App, ctx: &ReqCtx, _req: Empty) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    match (p.kind, p.credential_id) {
        (CredentialKind::Session, Some(id)) => {
            diesel::delete(sessions::table.find(id)).execute(&mut conn)?;
        }
        (CredentialKind::ApiToken, Some(id)) => {
            diesel::update(api_tokens::table.find(id))
                .set(api_tokens::revoked_at.eq(Utc::now()))
                .execute(&mut conn)?;
        }
        _ => return Err(Error::invalid("this credential cannot be revoked here")),
    }
    Ok(WireOk { ok: true })
}
