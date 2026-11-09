//! Who is calling. Resolved once per request from the carrier credential.

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_db::models::{ApiToken, Identity, Session};
use vcsstar_db::schema::{api_tokens, identities, sessions};
use vcsstar_transport::Peer;

use crate::tokens::{self, ACT_AS_PREFIX, API_TOKEN_PREFIX, SESSION_PREFIX};
use crate::{App, Code, Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Session,
    ApiToken,
    ActAs,
}

impl CredentialKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CredentialKind::Session => "session",
            CredentialKind::ApiToken => "api_token",
            CredentialKind::ActAs => "act_as",
        }
    }
}

/// An authenticated caller.
#[derive(Debug, Clone)]
pub struct Principal {
    pub identity: Identity,
    /// Scope strings; `*` means every scope (web sessions).
    pub scopes: Vec<String>,
    pub kind: CredentialKind,
    pub acting_via: Option<String>,
    pub credential_id: Option<uuid::Uuid>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

impl Principal {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes
            .iter()
            .any(|s| s == "*" || s == scope || implies(s, scope))
    }

    pub fn require_scope(&self, scope: &str) -> Result<(), Error> {
        if self.has_scope(scope) {
            Ok(())
        } else {
            Err(Error::forbidden(format!("credential lacks scope {scope}")))
        }
    }

    pub fn key(&self) -> String {
        format!("{}@{}", self.identity.lk_user_id, self.identity.lk_domain)
    }

    pub fn to_wire(&self) -> vcsstar_types::Identity {
        identity_to_wire(&self.identity)
    }
}

/// Scope implication: a write scope covers its read scope, `admin` covers all.
fn implies(held: &str, wanted: &str) -> bool {
    if held == "admin" {
        return true;
    }
    match (held.split_once(':'), wanted.split_once(':')) {
        (Some((hr, hl)), Some((wr, wl))) if hr == wr => {
            matches!((hl, wl), ("admin", _) | ("write", "read"))
        }
        _ => false,
    }
}

pub fn identity_to_wire(i: &Identity) -> vcsstar_types::Identity {
    vcsstar_types::Identity {
        user_id: i.lk_user_id.to_string(),
        domain: i.lk_domain.clone(),
        handle: Some(i.handle.clone()),
        display_name: i.display_name.clone(),
    }
}

/// Per-request context handed to every handler.
pub struct ReqCtx {
    pub request_id: String,
    pub principal: Option<Principal>,
    pub peer: Peer,
}

impl ReqCtx {
    pub fn require(&self) -> Result<&Principal, Error> {
        self.principal.as_ref().ok_or_else(Error::unauthenticated)
    }
    pub fn actor_id(&self) -> Option<uuid::Uuid> {
        self.principal.as_ref().map(|p| p.identity.id)
    }
}

/// Resolve the carrier credential to a principal. `None` means anonymous.
pub fn resolve(app: &App, peer: &Peer) -> Result<Option<Principal>, Error> {
    let Some(credential) = peer.credential.as_deref() else {
        return Ok(None);
    };
    let credential = credential
        .strip_prefix("Bearer ")
        .unwrap_or(credential)
        .trim();
    if credential.is_empty() {
        return Ok(None);
    }
    if credential.starts_with(API_TOKEN_PREFIX) {
        resolve_api_token(app, credential, CredentialKind::ApiToken).map(Some)
    } else if credential.starts_with(ACT_AS_PREFIX) {
        resolve_api_token(app, credential, CredentialKind::ActAs).map(Some)
    } else if credential.starts_with(SESSION_PREFIX) {
        resolve_session(app, credential).map(Some)
    } else {
        Err(Error::new(Code::Unauthenticated, "unrecognized credential"))
    }
}

fn resolve_api_token(app: &App, secret: &str, kind: CredentialKind) -> Result<Principal, Error> {
    let mut conn = app.conn()?;
    let digest = tokens::hash(secret);
    let (token, identity): (ApiToken, Identity) = api_tokens::table
        .inner_join(identities::table)
        .filter(api_tokens::token_hash.eq(&digest))
        .select((ApiToken::as_select(), Identity::as_select()))
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| Error::new(Code::Unauthenticated, "unknown token"))?;
    if token.revoked_at.is_some() {
        return Err(Error::new(Code::Unauthenticated, "token revoked"));
    }
    if let Some(exp) = token.expires_at {
        if exp <= Utc::now() {
            return Err(Error::new(Code::TokenExpired, "token expired"));
        }
    }
    if identity.disabled_at.is_some() {
        return Err(Error::forbidden("identity disabled"));
    }
    Ok(Principal {
        identity,
        scopes: token.scopes.into_iter().flatten().collect(),
        kind,
        acting_via: token.client_label.clone(),
        credential_id: Some(token.id),
        expires_at: token.expires_at,
    })
}

fn resolve_session(app: &App, secret: &str) -> Result<Principal, Error> {
    let mut conn = app.conn()?;
    let digest = tokens::hash(secret);
    let (session, identity): (Session, Identity) = sessions::table
        .inner_join(identities::table)
        .filter(sessions::token_hash.eq(&digest))
        .select((Session::as_select(), Identity::as_select()))
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| Error::new(Code::Unauthenticated, "unknown session"))?;
    if session.expires_at <= Utc::now() {
        return Err(Error::new(Code::TokenExpired, "session expired"));
    }
    if identity.disabled_at.is_some() {
        return Err(Error::forbidden("identity disabled"));
    }
    diesel::update(sessions::table.find(session.id))
        .set(sessions::last_used_at.eq(Utc::now()))
        .execute(&mut conn)
        .ok();
    Ok(Principal {
        identity,
        scopes: vec!["*".into()],
        kind: CredentialKind::Session,
        acting_via: None,
        credential_id: Some(session.id),
        expires_at: Some(session.expires_at),
    })
}

#[cfg(test)]
mod tests {
    use super::implies;

    #[test]
    fn scope_implication() {
        assert!(implies("repo:write", "repo:read"));
        assert!(implies("repo:admin", "repo:write"));
        assert!(implies("admin", "mirror:manage"));
        assert!(!implies("repo:read", "repo:write"));
        assert!(!implies("org:admin", "repo:read"));
    }
}
