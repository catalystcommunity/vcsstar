//! A vcsstar identity linked to a GitHub account. With a link, vcsstar
//! acts on GitHub *as that user*: pull requests, comments, reviews,
//! statuses, and branch pushes the user causes on vcsstar are sent with
//! the user's token, so GitHub shows them under the user's login. In the
//! other direction, a webhook whose sender is linked is attributed to the
//! vcsstar identity instead of a shadow identity.
//!
//! Two ways to link: the instance's GitHub App (user authorization,
//! `/auth/github/start` -> GitHub -> `/auth/github/callback`; expiring
//! tokens that vcsstar refreshes) or a personal access token the user
//! pastes (`Auth/set-github-token`). Both are sealed with the instance
//! credential key, like stored mirror credentials.

use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use serde_json::Value as Json;
use uuid::Uuid;
use vcsstar_db::models::Identity;
use vcsstar_db::schema::{
    github_identities, github_link_attempts, github_links, identities, pr_comments, pr_reviews,
    pull_requests, statuses,
};
use vcsstar_types::GithubLink;

use crate::config::GithubAppConfig;
use crate::github::{self, Failure, OauthTokens};
use crate::mirror::cred;
use crate::{to_ms, tokens, App, Code, Error};

const ATTEMPT_TTL_MINUTES: i64 = 15;
/// Refresh an access token this close to its expiry.
const REFRESH_MARGIN_MINUTES: i64 = 5;

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = github_links)]
pub struct LinkRow {
    pub identity_id: Uuid,
    pub github_id: i64,
    pub login: String,
    pub kind: String,
    pub access_ciphertext: Vec<u8>,
    pub access_expires_at: Option<DateTime<Utc>>,
    pub refresh_ciphertext: Option<Vec<u8>>,
    pub refresh_expires_at: Option<DateTime<Utc>>,
    pub broken_at: Option<DateTime<Utc>>,
    pub broken_reason: Option<String>,
    pub linked_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// The App whose client issued the token (NULL: the first App).
    pub app_id: Option<String>,
}

/// The GitHub App that links accounts (the first one), or a precondition
/// error for the caller.
pub fn app_config(app: &App) -> Result<&GithubAppConfig, Error> {
    app.config.github_app().ok_or_else(|| {
        Error::new(
            Code::PreconditionFailed,
            "this instance has no GitHub App; set SECRET_VCSSTAR__GITHUB_APPS (or _GITHUB_APP_*) or paste a personal access token",
        )
    })
}

pub fn callback_url(app: &App) -> String {
    format!(
        "{}/auth/github/callback",
        app.config.public_url.trim_end_matches('/')
    )
}

pub fn start_url(app: &App) -> Option<String> {
    app.config.github_app().map(|_| {
        format!(
            "{}/auth/github/start",
            app.config.public_url.trim_end_matches('/')
        )
    })
}

pub fn load(conn: &mut PgConnection, identity_id: Uuid) -> Result<Option<LinkRow>, Error> {
    Ok(github_links::table
        .find(identity_id)
        .select(LinkRow::as_select())
        .first(conn)
        .optional()?)
}

/// Step 1 of the App authorization: remember the attempt and answer the
/// GitHub authorize URL the browser is sent to.
pub fn begin(
    app: &App,
    conn: &mut PgConnection,
    identity_id: Uuid,
    return_to: Option<&str>,
) -> Result<String, Error> {
    let cfg = app_config(app)?;
    cred::require_key(app)?;
    let state = tokens::new_secret("vgl_");
    diesel::delete(
        github_link_attempts::table.filter(github_link_attempts::expires_at.lt(Utc::now())),
    )
    .execute(conn)?;
    diesel::insert_into(github_link_attempts::table)
        .values((
            github_link_attempts::state_hash.eq(tokens::hash(&state)),
            github_link_attempts::identity_id.eq(identity_id),
            github_link_attempts::return_to.eq(return_to),
            github_link_attempts::expires_at
                .eq(Utc::now() + Duration::minutes(ATTEMPT_TTL_MINUTES)),
        ))
        .execute(conn)?;
    Ok(format!(
        "{}/login/oauth/authorize?client_id={}&redirect_uri={}&state={}",
        app.config.github_web_base,
        crate::linkkeys::urlencode(&cfg.client_id),
        crate::linkkeys::urlencode(&callback_url(app)),
        crate::linkkeys::urlencode(&state)
    ))
}

/// Step 2: the callback. The state must name an attempt of the signed-in
/// identity; the code is exchanged, the account read, the link stored.
/// Returns the row and the `return_to` of the attempt.
pub fn complete(
    app: &App,
    conn: &mut PgConnection,
    identity_id: Uuid,
    state: &str,
    code: &str,
) -> Result<(LinkRow, Option<String>), Error> {
    let cfg = app_config(app)?;
    let attempt: Option<(Uuid, Option<String>, DateTime<Utc>)> = github_link_attempts::table
        .find(tokens::hash(state))
        .select((
            github_link_attempts::identity_id,
            github_link_attempts::return_to,
            github_link_attempts::expires_at,
        ))
        .first(conn)
        .optional()?;
    let Some((attempt_identity, return_to, expires_at)) = attempt else {
        return Err(Error::new(
            Code::PreconditionFailed,
            "this GitHub authorization was not started here, or it expired; start again",
        ));
    };
    diesel::delete(github_link_attempts::table.find(tokens::hash(state))).execute(conn)?;
    if expires_at < Utc::now() {
        return Err(Error::new(
            Code::PreconditionFailed,
            "the GitHub authorization took too long; start again",
        ));
    }
    if attempt_identity != identity_id {
        return Err(Error::forbidden(
            "this GitHub authorization was started by a different account",
        ));
    }
    let tok = github::oauth_exchange(
        &app.config.github_web_base,
        &cfg.client_id,
        &cfg.client_secret,
        code,
        &callback_url(app),
    )?;
    let (gid, login) =
        github::Client::new(&app.config.github_api_base, &tok.access_token).get_user()?;
    store(app, conn, identity_id, gid, &login, "oauth", &tok)?;
    // Remember the issuing App: a refresh must use its client.
    let row: LinkRow = diesel::update(github_links::table.find(identity_id))
        .set(github_links::app_id.eq(Some(cfg.app_id.as_str())))
        .returning(LinkRow::as_returning())
        .get_result(conn)?;
    Ok((row, return_to))
}

/// Link with a personal access token the user pasted.
pub fn set_token(
    app: &App,
    conn: &mut PgConnection,
    identity_id: Uuid,
    token: &str,
) -> Result<LinkRow, Error> {
    let token = token.trim();
    if token.is_empty() || token.len() > 400 || !token.is_ascii() {
        return Err(Error::invalid("a GitHub token is a short ASCII string"));
    }
    let (gid, login) = github::Client::new(&app.config.github_api_base, token).get_user()?;
    let tok = OauthTokens {
        access_token: token.to_string(),
        expires_at: None,
        refresh_token: None,
        refresh_expires_at: None,
    };
    store(app, conn, identity_id, gid, &login, "pat", &tok)
}

/// Store a link whose account was verified by the caller (the callback
/// and `set_token` read `/user` first). Rows the shadow identity of that
/// GitHub account owned move to the linked identity.
pub fn store(
    app: &App,
    conn: &mut PgConnection,
    identity_id: Uuid,
    github_id: i64,
    login: &str,
    kind: &str,
    tok: &OauthTokens,
) -> Result<LinkRow, Error> {
    let other: Option<Uuid> = github_links::table
        .filter(github_links::github_id.eq(github_id))
        .filter(github_links::identity_id.ne(identity_id))
        .select(github_links::identity_id)
        .first(conn)
        .optional()?;
    if other.is_some() {
        return Err(Error::new(
            Code::Conflict,
            format!("GitHub account {login} is already linked to another account here"),
        ));
    }
    let access = cred::seal(app, tok.access_token.as_bytes())?;
    let refresh = match &tok.refresh_token {
        Some(r) => Some(cred::seal(app, r.as_bytes())?),
        None => None,
    };
    let row: LinkRow = conn.transaction::<LinkRow, Error, _>(|conn| {
        let row: LinkRow = diesel::insert_into(github_links::table)
            .values((
                github_links::identity_id.eq(identity_id),
                github_links::github_id.eq(github_id),
                github_links::login.eq(login),
                github_links::kind.eq(kind),
                github_links::access_ciphertext.eq(&access),
                github_links::access_expires_at.eq(tok.expires_at),
                github_links::refresh_ciphertext.eq(&refresh),
                github_links::refresh_expires_at.eq(tok.refresh_expires_at),
            ))
            .on_conflict(github_links::identity_id)
            .do_update()
            .set((
                github_links::github_id.eq(github_id),
                github_links::login.eq(login),
                github_links::kind.eq(kind),
                github_links::access_ciphertext.eq(&access),
                github_links::access_expires_at.eq(tok.expires_at),
                github_links::refresh_ciphertext.eq(&refresh),
                github_links::refresh_expires_at.eq(tok.refresh_expires_at),
                github_links::broken_at.eq(None::<DateTime<Utc>>),
                github_links::broken_reason.eq(None::<String>),
                github_links::app_id.eq(None::<String>),
                github_links::updated_at.eq(Utc::now()),
            ))
            .returning(LinkRow::as_returning())
            .get_result(conn)?;
        absorb_shadow(conn, identity_id, github_id)?;
        Ok(row)
    })?;
    Ok(row)
}

/// What the GitHub account did before the link was attributed to its
/// shadow identity; move those rows to the real identity. The shadow row
/// stays (events already reference it) but owns nothing any more.
fn absorb_shadow(
    conn: &mut PgConnection,
    identity_id: Uuid,
    github_id: i64,
) -> Result<usize, Error> {
    let shadow: Option<Uuid> = github_identities::table
        .filter(github_identities::github_id.eq(github_id))
        .select(github_identities::identity_id)
        .first(conn)
        .optional()?;
    let Some(shadow) = shadow else {
        return Ok(0);
    };
    if shadow == identity_id {
        return Ok(0);
    }
    let mut n = 0;
    n += diesel::update(pull_requests::table.filter(pull_requests::author_identity_id.eq(shadow)))
        .set(pull_requests::author_identity_id.eq(identity_id))
        .execute(conn)?;
    n += diesel::update(pull_requests::table.filter(pull_requests::merged_by.eq(shadow)))
        .set(pull_requests::merged_by.eq(identity_id))
        .execute(conn)?;
    n += diesel::update(pr_reviews::table.filter(pr_reviews::reviewer.eq(shadow)))
        .set(pr_reviews::reviewer.eq(identity_id))
        .execute(conn)?;
    n += diesel::update(pr_comments::table.filter(pr_comments::author.eq(shadow)))
        .set(pr_comments::author.eq(identity_id))
        .execute(conn)?;
    n += diesel::update(statuses::table.filter(statuses::reporter.eq(shadow)))
        .set(statuses::reporter.eq(identity_id))
        .execute(conn)?;
    Ok(n)
}

pub fn unlink(app: &App, conn: &mut PgConnection, identity_id: Uuid) -> Result<bool, Error> {
    let Some(row) = load(conn, identity_id)? else {
        return Ok(false);
    };
    if row.kind == "oauth" {
        if let (Some(cfg), Ok(token)) =
            (link_app(app, &row), cred::open(app, &row.access_ciphertext))
        {
            if let Err(e) = github::revoke_grant(
                &app.config.github_api_base,
                &cfg.client_id,
                &cfg.client_secret,
                &token,
            ) {
                tracing::warn!(error = %e.message(), "GitHub grant revocation failed; the link is removed anyway");
            }
        }
    }
    diesel::delete(github_links::table.find(identity_id)).execute(conn)?;
    Ok(true)
}

pub fn status(app: &App, conn: &mut PgConnection, identity_id: Uuid) -> Result<GithubLink, Error> {
    let row = load(conn, identity_id)?;
    Ok(to_wire(app, row.as_ref()))
}

pub fn to_wire(app: &App, row: Option<&LinkRow>) -> GithubLink {
    let slug = app.config.github_app().and_then(|c| c.slug.clone());
    match row {
        Some(r) => GithubLink {
            linked: true,
            kind: Some(r.kind.clone()),
            login: Some(r.login.clone()),
            github_id: Some(r.github_id.max(0) as u64),
            linked_at: Some(to_ms(r.linked_at)),
            expires_at: r.access_expires_at.map(to_ms),
            broken_reason: r.broken_reason.clone(),
            start_url: start_url(app),
            app_slug: slug,
        },
        None => GithubLink {
            linked: false,
            kind: None,
            login: None,
            github_id: None,
            linked_at: None,
            expires_at: None,
            broken_reason: None,
            start_url: start_url(app),
            app_slug: slug,
        },
    }
}

/// The vcsstar identity a GitHub account is linked to, if any.
pub fn identity_for_github_id(
    conn: &mut PgConnection,
    github_id: i64,
) -> Result<Option<Identity>, Error> {
    Ok(github_links::table
        .inner_join(identities::table)
        .filter(github_links::github_id.eq(github_id))
        .select(Identity::as_select())
        .first(conn)
        .optional()?)
}

/// The identity a GitHub `user` object (`{id, login}`) stands for: the
/// linked vcsstar identity when there is one, else the shadow identity.
pub fn identity_for_user(conn: &mut PgConnection, user: &Json) -> Result<Identity, Error> {
    if let Some(id) = user.get("id").and_then(Json::as_i64) {
        if let Some(i) = identity_for_github_id(conn, id)? {
            return Ok(i);
        }
    }
    crate::github_sync::shadow_identity(conn, user)
}

/// Record that GitHub refused the user's token; the mirror falls back to
/// the instance App or organization credential until the user connects
/// again.
pub fn mark_broken(conn: &mut PgConnection, identity_id: Uuid, reason: &str) {
    let reason: String = reason.chars().take(300).collect();
    let _ = diesel::update(github_links::table.find(identity_id))
        .set((
            github_links::broken_at.eq(Some(Utc::now())),
            github_links::broken_reason.eq(Some(reason)),
            github_links::updated_at.eq(Utc::now()),
        ))
        .execute(conn);
}

/// A usable token for a linked identity: `Ok(None)` when the identity is
/// not linked or its link is broken; refreshed first when it is about to
/// expire. Returns `(login, token)`.
pub fn user_token(
    app: &App,
    conn: &mut PgConnection,
    identity_id: Uuid,
) -> Result<Option<(String, String)>, Failure> {
    let Some(row) = load(conn, identity_id).map_err(|e| Failure::Transient(e.message))? else {
        return Ok(None);
    };
    if row.broken_at.is_some() {
        return Ok(None);
    }
    let fresh = row
        .access_expires_at
        .is_none_or(|t| t - Duration::minutes(REFRESH_MARGIN_MINUTES) > Utc::now());
    let token = if fresh {
        cred::open(app, &row.access_ciphertext).map_err(|e| Failure::Terminal(e.message))?
    } else {
        match refresh(app, conn, &row) {
            Ok(t) => t,
            Err(Failure::Terminal(m)) => {
                tracing::warn!(identity = %identity_id, login = %row.login, error = %m, "GitHub user token refresh refused; link marked broken");
                mark_broken(conn, identity_id, &m);
                return Ok(None);
            }
            Err(f) => return Err(f),
        }
    };
    let _ = diesel::update(github_links::table.find(identity_id))
        .set(github_links::last_used_at.eq(Some(Utc::now())))
        .execute(conn);
    Ok(Some((row.login, token)))
}

/// The App whose client issued a link's token: the recorded one, else the
/// first App (links made before the App was recorded).
fn link_app<'a>(app: &'a App, row: &LinkRow) -> Option<&'a GithubAppConfig> {
    match row.app_id.as_deref() {
        Some(id) => app.config.github_app_by_id(id),
        None => app.config.github_app(),
    }
}

fn refresh(app: &App, conn: &mut PgConnection, row: &LinkRow) -> Result<String, Failure> {
    let cfg = link_app(app, row).ok_or_else(|| {
        Failure::Terminal("the GitHub App that issued this link is no longer configured".into())
    })?;
    let refresh_ct = row.refresh_ciphertext.as_ref().ok_or_else(|| {
        Failure::Terminal("the access token expired and there is no refresh token".into())
    })?;
    if row.refresh_expires_at.is_some_and(|t| t <= Utc::now()) {
        return Err(Failure::Terminal("the refresh token expired".into()));
    }
    let refresh_token = cred::open(app, refresh_ct).map_err(|e| Failure::Terminal(e.message))?;
    let tok = github::oauth_refresh(
        &app.config.github_web_base,
        &cfg.client_id,
        &cfg.client_secret,
        &refresh_token,
    )?;
    let access =
        cred::seal(app, tok.access_token.as_bytes()).map_err(|e| Failure::Terminal(e.message))?;
    let new_refresh = match &tok.refresh_token {
        Some(r) => Some(cred::seal(app, r.as_bytes()).map_err(|e| Failure::Terminal(e.message))?),
        None => None,
    };
    diesel::update(github_links::table.find(row.identity_id))
        .set((
            github_links::access_ciphertext.eq(&access),
            github_links::access_expires_at.eq(tok.expires_at),
            github_links::refresh_ciphertext
                .eq(new_refresh.as_ref().or(row.refresh_ciphertext.as_ref())),
            github_links::refresh_expires_at.eq(tok.refresh_expires_at.or(row.refresh_expires_at)),
            github_links::updated_at.eq(Utc::now()),
        ))
        .execute(conn)
        .map_err(|e| Failure::Transient(format!("database: {e}")))?;
    Ok(tok.access_token)
}

#[cfg(test)]
mod tests {
    #[test]
    fn freshness_margin() {
        use chrono::{Duration, Utc};
        let soon = Utc::now() + Duration::minutes(2);
        let later = Utc::now() + Duration::hours(1);
        let fresh = |t: Option<chrono::DateTime<Utc>>| {
            t.is_none_or(|t| t - Duration::minutes(super::REFRESH_MARGIN_MINUTES) > Utc::now())
        };
        assert!(fresh(None));
        assert!(!fresh(Some(soon)));
        assert!(fresh(Some(later)));
    }
}
