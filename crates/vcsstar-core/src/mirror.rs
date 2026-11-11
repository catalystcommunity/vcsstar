//! The GitHub mirror engine: credentials, per-object-key actions with the
//! newest desired state, convergence (push or API upsert), failure
//! classification, drift handling, reconcile, and import. Workers call
//! `execute`; the dispatcher calls `expand_event`; the Mirror service ops
//! read and replay the rows.

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vcsstar_db::models::{Org, Repo};
use vcsstar_db::schema::{
    credentials, events, mirror_actions, mirror_attempts, mirror_refs, mirror_state, orgs, repos,
};
use vcsstar_db::DbConn;
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_types::{decode_event, EventPayload, MirrorPayload};
use vcsstar_vcs::{Env, RevId};

use crate::github::{self, Failure};
use crate::outbox::{self, Emit};
use crate::{push, vcs, App, Code, Error};

pub const PURPOSES: &[&str] = &[
    "mirror_git_push",
    "mirror_api",
    "import_fetch",
    "webhook_inbound",
];
pub const MODES: &[&str] = &["authoritative_vcsstar", "import_drift", "flag_drift"];

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = mirror_state)]
pub struct StateRow {
    pub repo_id: Uuid,
    pub github_repo: String,
    pub mode: String,
    pub paused: bool,
    pub cursor_seq: i64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_full_reconcile_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_pull_sync_at: Option<DateTime<Utc>>,
    pub last_pull_pass_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = mirror_actions)]
pub struct ActionRow {
    pub id: Uuid,
    pub repo_id: Uuid,
    pub event_lane: String,
    pub event_seq: i64,
    pub action_kind: String,
    pub object_key: String,
    pub state: String,
    pub blocked_reason: Option<String>,
    pub blocked_at: Option<DateTime<Utc>>,
    pub credential_purpose: String,
    pub payload_cbor: Vec<u8>,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = credentials)]
pub struct CredentialRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub repo_id: Option<Uuid>,
    pub provider: String,
    pub purpose: String,
    pub auth_method: String,
    pub host: String,
    pub secret_ref: String,
    pub secret_ciphertext: Option<Vec<u8>>,
    pub enabled: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// What an action converges. The ref target is read from Postgres at
/// execution time, so the payload names the object, not the event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionPayload {
    Push {
        r#ref: String,
    },
    RepoSettings,
    /// Converge the GitHub pull request that shadows vcsstar PR `number`
    /// (create, title, body, base, state).
    PrSync {
        number: i32,
    },
    /// Post or edit one vcsstar comment on the GitHub pull request.
    PrComment {
        id: Uuid,
    },
    /// Submit one vcsstar review on the GitHub pull request.
    PrReview {
        id: Uuid,
    },
    /// Report one commit status to GitHub.
    Status {
        sha: String,
        context: String,
    },
}

impl ActionPayload {
    /// Object kind for metrics and logs.
    pub fn kind(&self) -> &'static str {
        match self {
            ActionPayload::Push { .. } => "ref",
            ActionPayload::RepoSettings => "repo-settings",
            ActionPayload::PrSync { .. } => "pr",
            ActionPayload::PrComment { .. } => "pr-comment",
            ActionPayload::PrReview { .. } => "pr-review",
            ActionPayload::Status { .. } => "status",
        }
    }
}

pub fn encode_payload(p: &ActionPayload) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(p, &mut out).expect("cbor");
    out
}

pub fn decode_payload(b: &[u8]) -> Option<ActionPayload> {
    ciborium::de::from_reader(b).ok()
}

pub fn queue_name(org: &str, repo: &str) -> String {
    format!("mirror:{org}/{repo}")
}

/// The `mirror_state` table, for modules that update one column of it.
pub fn mirror_state_table() -> mirror_state::table {
    mirror_state::table
}

pub fn state_for(conn: &mut PgConnection, repo_id: Uuid) -> Result<Option<StateRow>, Error> {
    Ok(mirror_state::table
        .find(repo_id)
        .select(StateRow::as_select())
        .first(conn)
        .optional()?)
}

// ---------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------

pub mod cred {
    use super::*;
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};

    pub fn seal(app: &App, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let key = app.config.credential_key.ok_or_else(|| {
            Error::new(
                Code::PreconditionFailed,
                "this instance has no SECRET_VCSSTAR__CREDENTIAL_KEY; use an env: secret_ref",
            )
        })?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let mut nonce = [0u8; 24];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
        let ct = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext)
            .map_err(|_| Error::internal("seal failed"))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        Ok(out)
    }

    pub fn open(app: &App, sealed: &[u8]) -> Result<String, Error> {
        let key = app
            .config
            .credential_key
            .ok_or_else(|| Error::new(Code::PreconditionFailed, "credential key not configured"))?;
        if sealed.len() < 24 {
            return Err(Error::internal("sealed credential too short"));
        }
        let cipher = XChaCha20Poly1305::new((&key).into());
        let pt = cipher
            .decrypt(XNonce::from_slice(&sealed[..24]), &sealed[24..])
            .map_err(|_| Error::internal("credential does not open with this key"))?;
        String::from_utf8(pt).map_err(|_| Error::internal("credential is not text"))
    }

    /// The secret value a credential row points at.
    pub fn value(app: &App, row: &CredentialRow) -> Result<String, Error> {
        if let Some(var) = row.secret_ref.strip_prefix("env:") {
            return std::env::var(var)
                .map(|v| v.trim().to_string())
                .map_err(|_| {
                    Error::new(
                        Code::PreconditionFailed,
                        format!("credential {} points at unset variable {var}", row.id),
                    )
                });
        }
        match &row.secret_ciphertext {
            Some(ct) => open(app, ct),
            None => Err(Error::new(
                Code::PreconditionFailed,
                format!("credential {} has no stored secret", row.id),
            )),
        }
    }

    /// The credential set for a purpose: every enabled row at the most
    /// specific scope (the repository's own rows when it has any, else the
    /// organization's).
    pub fn candidates(
        conn: &mut PgConnection,
        org_id: Uuid,
        repo_id: Uuid,
        purpose: &str,
    ) -> Result<Vec<CredentialRow>, Error> {
        let rows: Vec<CredentialRow> = credentials::table
            .filter(credentials::org_id.eq(org_id))
            .filter(credentials::purpose.eq(purpose))
            .filter(credentials::enabled.eq(true))
            .filter(
                credentials::repo_id
                    .eq(repo_id)
                    .or(credentials::repo_id.is_null()),
            )
            .select(CredentialRow::as_select())
            .load(conn)?;
        let (repo_rows, org_rows): (Vec<_>, Vec<_>) =
            rows.into_iter().partition(|r| r.repo_id == Some(repo_id));
        Ok(if repo_rows.is_empty() {
            org_rows
        } else {
            repo_rows
        })
    }

    /// One member of the credential set, chosen at random so several
    /// tokens or Apps share the load.
    pub fn find(
        conn: &mut PgConnection,
        org_id: Uuid,
        repo_id: Uuid,
        purpose: &str,
    ) -> Result<Option<CredentialRow>, Error> {
        use rand::seq::SliceRandom;
        let set = candidates(conn, org_id, repo_id, purpose)?;
        Ok(set.choose(&mut rand::thread_rng()).cloned())
    }

    /// Where a token came from. The mirror tells GitHub nothing about
    /// it, but the executors word what they post by it (a user's own
    /// token needs no attribution line) and settle refusals by it.
    #[derive(Debug, Clone)]
    pub enum TokenSource {
        /// A linked user's own token: GitHub attributes the change to them.
        User { identity_id: Uuid, login: String },
        /// The instance's GitHub App, installed on the repository's owner.
        InstanceApp,
        /// A credential row of the organization or repository.
        Credential { purpose: String, id: Uuid },
    }

    #[derive(Debug, Clone)]
    pub struct Token {
        pub value: String,
        pub source: TokenSource,
    }

    impl Token {
        pub fn is_user(&self) -> bool {
            matches!(self.source, TokenSource::User { .. })
        }

        /// A short label for logs: `user:<login>`, `app`, or the purpose.
        pub fn label(&self) -> String {
            match &self.source {
                TokenSource::User { login, .. } => format!("user:{login}"),
                TokenSource::InstanceApp => "app".into(),
                TokenSource::Credential { purpose, .. } => purpose.clone(),
            }
        }
    }

    /// Sealing needs the instance key; say so before a token is fetched.
    pub fn require_key(app: &App) -> Result<(), Error> {
        if app.config.credential_key.is_none() {
            return Err(Error::new(
                Code::PreconditionFailed,
                "this instance has no SECRET_VCSSTAR__CREDENTIAL_KEY, so it cannot keep a token",
            ));
        }
        Ok(())
    }

    fn instance_app_key(full_name: &str) -> Uuid {
        Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("vcsstar:github-app:{full_name}").as_bytes(),
        )
    }

    /// The GitHub repository a mirrored repository shadows.
    fn github_repo_of(conn: &mut PgConnection, repo_id: Uuid) -> Result<Option<String>, Failure> {
        mirror_state::table
            .filter(mirror_state::repo_id.eq(repo_id))
            .select(mirror_state::github_repo)
            .first::<String>(conn)
            .optional()
            .map_err(|e| Failure::Transient(format!("database: {e}")))
    }

    /// An installation token of the instance's GitHub App for the
    /// repository, when the App is configured and installed on its owner
    /// for that repository. Cached per repository for the token's life;
    /// a missing installation is remembered for a while.
    pub fn instance_app_token(
        app: &App,
        conn: &mut PgConnection,
        repo_id: Uuid,
    ) -> Result<Option<String>, Failure> {
        let Some(full) = github_repo_of(conn, repo_id)? else {
            return Ok(None);
        };
        instance_app_token_for(app, &full)
    }

    /// `instance_app_token` by GitHub repository name, for operations that
    /// run before a vcsstar repository exists (import, discovery).
    pub fn instance_app_token_for(app: &App, full: &str) -> Result<Option<String>, Failure> {
        let Some(cfg) = app.config.github_app.as_ref() else {
            return Ok(None);
        };
        let key = instance_app_key(full);
        if let Some(t) = app.github_app_tokens.get(key) {
            return Ok(Some(t));
        }
        if app.github_app_tokens.not_installed(full) {
            return Ok(None);
        }
        let jwt = crate::github_app::sign_jwt(&cfg.app_id, &cfg.private_key)
            .map_err(|e| Failure::Terminal(e.message))?;
        let Some(installation) =
            github::installation_for_repo(&app.config.github_api_base, &jwt, full)?
        else {
            // GitHub answers 404 for a while after an installation is
            // made, so this is remembered only briefly.
            tracing::info!(repo = %full, "the instance GitHub App is not installed for this repository (or GitHub does not show it yet)");
            app.github_app_tokens.remember_not_installed(full);
            return Ok(None);
        };
        let (token, expires_at) = github::installation_token(
            &app.config.github_api_base,
            &jwt,
            &installation.to_string(),
        )?;
        app.github_app_tokens.put(key, token.clone(), expires_at);
        Ok(Some(token))
    }

    /// An installation token of the instance App on an owner (every
    /// repository the installation covers), with the installation id.
    pub fn owner_app_token(app: &App, owner: &str) -> Result<Option<(i64, String)>, Failure> {
        let Some(cfg) = app.config.github_app.as_ref() else {
            return Ok(None);
        };
        let cache_name = format!("owner:{owner}");
        let key = instance_app_key(&cache_name);
        let id_key = instance_app_key(&format!("{cache_name}:id"));
        if let (Some(t), Some(id)) = (
            app.github_app_tokens.get(key),
            app.github_app_tokens.get(id_key),
        ) {
            if let Ok(id) = id.parse::<i64>() {
                return Ok(Some((id, t)));
            }
        }
        if app.github_app_tokens.not_installed(&cache_name) {
            return Ok(None);
        }
        let jwt = crate::github_app::sign_jwt(&cfg.app_id, &cfg.private_key)
            .map_err(|e| Failure::Terminal(e.message))?;
        let Some(installation) =
            github::installation_for_owner(&app.config.github_api_base, &jwt, owner)?
        else {
            tracing::info!(
                owner,
                "the instance GitHub App is not installed on this owner"
            );
            app.github_app_tokens.remember_not_installed(&cache_name);
            return Ok(None);
        };
        let (token, expires_at) = github::installation_token(
            &app.config.github_api_base,
            &jwt,
            &installation.to_string(),
        )?;
        app.github_app_tokens.put(key, token.clone(), expires_at);
        app.github_app_tokens
            .put(id_key, installation.to_string(), expires_at);
        Ok(Some((installation, token)))
    }

    /// The token for reading an owner's repositories or importing one of
    /// them, in order: the instance App's installation on the owner, the
    /// organization's credential set, then the caller's own linked token.
    /// Returns the installation id when the App was used.
    pub fn token_for_owner(
        app: &App,
        conn: &mut PgConnection,
        org_id: Uuid,
        owner: &str,
        caller: Option<Uuid>,
    ) -> Result<(Token, Option<i64>), Failure> {
        if let Some((installation, value)) = owner_app_token(app, owner)? {
            return Ok((
                Token {
                    value,
                    source: TokenSource::InstanceApp,
                },
                Some(installation),
            ));
        }
        for p in purpose_order("import_fetch") {
            if let Some(row) =
                find(conn, org_id, Uuid::nil(), p).map_err(|e| Failure::Transient(e.message))?
            {
                let v = value(app, &row).map_err(|e| Failure::Terminal(e.message))?;
                let value = if row.auth_method == "github_app" {
                    crate::github_app::installation_token(app, row.id, &v)?
                } else {
                    v
                };
                return Ok((
                    Token {
                        value,
                        source: TokenSource::Credential {
                            purpose: p.to_string(),
                            id: row.id,
                        },
                    },
                    None,
                ));
            }
        }
        if let Some(identity_id) = caller {
            if let Some((login, value)) = crate::github_link::user_token(app, conn, identity_id)? {
                return Ok((
                    Token {
                        value,
                        source: TokenSource::User { identity_id, login },
                    },
                    None,
                ));
            }
        }
        Err(Failure::Terminal(format!(
            "no way to read {owner} on GitHub: the instance GitHub App is not installed there, the organization has no credential, and your account is not linked to GitHub"
        )))
    }

    /// The token to act with, in order: the actor's own token when the
    /// actor is linked to GitHub (so GitHub shows the change under their
    /// login), the instance's GitHub App when it is installed on the
    /// owner, then the credential set for the purpose.
    pub fn token_for_actor(
        app: &App,
        conn: &mut PgConnection,
        org_id: Uuid,
        repo_id: Uuid,
        purpose: &str,
        actor: Option<Uuid>,
    ) -> Result<Token, Failure> {
        if let Some(identity_id) = actor {
            if let Some((login, value)) = crate::github_link::user_token(app, conn, identity_id)? {
                return Ok(Token {
                    value,
                    source: TokenSource::User { identity_id, login },
                });
            }
        }
        if purpose != "webhook_inbound" {
            if let Some(value) = instance_app_token(app, conn, repo_id)? {
                return Ok(Token {
                    value,
                    source: TokenSource::InstanceApp,
                });
            }
        }
        for p in purpose_order(purpose) {
            if let Some(row) =
                find(conn, org_id, repo_id, p).map_err(|e| Failure::Transient(e.message))?
            {
                let v = value(app, &row).map_err(|e| Failure::Terminal(e.message))?;
                let value = if row.auth_method == "github_app" {
                    crate::github_app::installation_token(app, row.id, &v)?
                } else {
                    v
                };
                return Ok(Token {
                    value,
                    source: TokenSource::Credential {
                        purpose: p.to_string(),
                        id: row.id,
                    },
                });
            }
        }
        Err(Failure::Terminal(format!(
            "no enabled credential with purpose {purpose} for this repository or its organization, and the instance GitHub App is not installed on it"
        )))
    }

    /// Purposes tried for a request, in order: api <-> git_push share
    /// credentials, and an import may use either.
    fn purpose_order(purpose: &str) -> Vec<&str> {
        match purpose {
            "mirror_api" => vec!["mirror_api", "mirror_git_push"],
            "mirror_git_push" => vec!["mirror_git_push", "mirror_api"],
            "import_fetch" => vec!["import_fetch", "mirror_git_push", "mirror_api"],
            p => vec![p],
        }
    }

    /// After a terminal GitHub failure: drop the cached installation token
    /// of a `github_app` credential, so a replay mints a fresh one instead
    /// of presenting a token GitHub already refused.
    pub fn forget_app_token(
        app: &App,
        conn: &mut PgConnection,
        org_id: Uuid,
        repo_id: Uuid,
        purpose: &str,
    ) {
        if let Ok(Some(full)) = github_repo_of(conn, repo_id) {
            app.github_app_tokens.forget(instance_app_key(&full));
            app.github_app_tokens.forget_not_installed(&full);
        }
        for p in purpose_order(purpose) {
            if let Ok(set) = candidates(conn, org_id, repo_id, p) {
                if set.is_empty() {
                    continue;
                }
                for row in set.iter().filter(|r| r.auth_method == "github_app") {
                    app.github_app_tokens.forget(row.id);
                }
                return;
            }
        }
    }

    /// Token for a purpose without an actor: the instance App, then the
    /// credential set (`token_for_actor`). Returns `(label, token)`.
    pub fn token_for(
        app: &App,
        conn: &mut PgConnection,
        org_id: Uuid,
        repo_id: Uuid,
        purpose: &str,
    ) -> Result<(String, String), Failure> {
        token_for_actor(app, conn, org_id, repo_id, purpose, None).map(|t| (t.label(), t.value))
    }
}

// ---------------------------------------------------------------------
// Expansion: outbox event -> pending action per object key
// ---------------------------------------------------------------------

pub struct EventRef<'a> {
    pub repo_id: Uuid,
    pub lane: &'a str,
    pub seq: i64,
    pub kind: &'a str,
    pub payload_cbor: &'a [u8],
}

pub struct ActionSpec {
    pub kind: &'static str,
    pub object_key: String,
    pub purpose: &'static str,
    pub payload: ActionPayload,
}

/// Which actions an event implies. Empty for events the mirror ignores.
/// Events caused by GitHub itself (shadow actors, see `github_sync`) are
/// filtered by the dispatcher before this is called.
pub fn expand_event(ev: &EventRef<'_>) -> Vec<ActionSpec> {
    let decoded = decode_event(ev.payload_cbor).ok();
    let ref_name = decoded.as_ref().and_then(|e| match &e.payload {
        EventPayload::Variant0(p) => Some(p.r#ref.clone()),
        EventPayload::Variant1(p) => Some(p.r#ref.clone()),
        _ => None,
    });
    let pr = decoded.as_ref().and_then(|e| match &e.payload {
        EventPayload::Variant2(p) => Some(p.clone()),
        _ => None,
    });
    let api = |object_key: String, payload: ActionPayload| {
        vec![ActionSpec {
            kind: "api_call",
            object_key,
            purpose: "mirror_api",
            payload,
        }]
    };
    match ev.kind {
        "pr_opened" | "pr_updated" | "pr_closed" | "pr_reopened" | "pr_retargeted"
        | "pr_merged" | "pr_head_moved" => pr
            .map(|p| {
                let number = p.number.min(i32::MAX as u64) as i32;
                api(format!("pr:{number}"), ActionPayload::PrSync { number })
            })
            .unwrap_or_default(),
        "pr_comment" | "pr_comment_edited" => pr
            .and_then(|p| p.comment_id)
            .and_then(|id| Uuid::parse_str(&id).ok())
            .map(|id| api(format!("pr-comment:{id}"), ActionPayload::PrComment { id }))
            .unwrap_or_default(),
        "pr_review" => pr
            .and_then(|p| p.review_id)
            .and_then(|id| Uuid::parse_str(&id).ok())
            .map(|id| api(format!("pr-review:{id}"), ActionPayload::PrReview { id }))
            .unwrap_or_default(),
        "status_set" => decoded
            .as_ref()
            .and_then(|e| match &e.payload {
                EventPayload::Variant6(p) => Some(api(
                    format!("status:{}:{}", p.sha, p.context),
                    ActionPayload::Status {
                        sha: p.sha.clone(),
                        context: p.context.clone(),
                    },
                )),
                _ => None,
            })
            .unwrap_or_default(),
        "push" | "ref_create" | "ref_delete" | "tag_create" | "tag_delete" => ref_name
            .filter(|r| r.starts_with("refs/heads/") || r.starts_with("refs/tags/"))
            .map(|r| {
                vec![ActionSpec {
                    kind: "git_push",
                    object_key: r.clone(),
                    purpose: "mirror_git_push",
                    payload: ActionPayload::Push { r#ref: r },
                }]
            })
            .unwrap_or_default(),
        "repo_created" | "repo_settings_changed" | "repo_renamed" | "repo_archived" => {
            vec![ActionSpec {
                kind: "api_call",
                object_key: "repo-settings".into(),
                purpose: "mirror_api",
                payload: ActionPayload::RepoSettings,
            }]
        }
        _ => vec![],
    }
}

/// Insert or refresh the one pending action for the object key. Returns
/// the action id to submit a task for.
pub fn upsert_action(
    conn: &mut PgConnection,
    ev: &EventRef<'_>,
    spec: &ActionSpec,
) -> Result<Uuid, Error> {
    let existing: Option<Uuid> = mirror_actions::table
        .filter(mirror_actions::repo_id.eq(ev.repo_id))
        .filter(mirror_actions::object_key.eq(&spec.object_key))
        .filter(mirror_actions::state.eq("pending"))
        .select(mirror_actions::id)
        .first(conn)
        .optional()?;
    if let Some(id) = existing {
        diesel::update(mirror_actions::table.find(id))
            .set((
                mirror_actions::event_lane.eq(ev.lane),
                mirror_actions::event_seq.eq(ev.seq),
                mirror_actions::payload_cbor.eq(encode_payload(&spec.payload)),
                mirror_actions::updated_at.eq(Utc::now()),
            ))
            .execute(conn)?;
        return Ok(id);
    }
    let id = vcsstar_db::new_id();
    diesel::insert_into(mirror_actions::table)
        .values((
            mirror_actions::id.eq(id),
            mirror_actions::repo_id.eq(ev.repo_id),
            mirror_actions::event_lane.eq(ev.lane),
            mirror_actions::event_seq.eq(ev.seq),
            mirror_actions::action_kind.eq(spec.kind),
            mirror_actions::object_key.eq(&spec.object_key),
            mirror_actions::state.eq("pending"),
            mirror_actions::credential_purpose.eq(spec.purpose),
            mirror_actions::payload_cbor.eq(encode_payload(&spec.payload)),
        ))
        .execute(conn)?;
    diesel::update(mirror_state::table.find(ev.repo_id))
        .set(mirror_state::cursor_seq.eq(ev.seq))
        .execute(conn)?;
    Ok(id)
}

// ---------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------

#[derive(Debug)]
pub enum Exec {
    Applied,
    Skipped(String),
    Blocked(String),
    Transient(String),
    /// Not attempted: this node holds calls on the GitHub lane the action
    /// needs (see `github_health`). The worker parks the task for about
    /// this long; no attempt is recorded.
    Held(std::time::Duration),
}

fn load_repo(conn: &mut PgConnection, repo_id: Uuid) -> Result<(Org, Repo), Error> {
    let repo: Repo = repos::table
        .find(repo_id)
        .select(Repo::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("repository"))?;
    let org: Org = orgs::table
        .find(repo.org_id)
        .select(Org::as_select())
        .first(conn)?;
    Ok((org, repo))
}

/// The actor of the event an action was made for, when it had one.
pub fn event_actor(conn: &mut PgConnection, action: &ActionRow) -> Result<Option<Uuid>, Error> {
    if action.event_seq <= 0 {
        return Ok(None);
    }
    Ok(events::table
        .filter(events::lane.eq(&action.event_lane))
        .filter(events::seq.eq(action.event_seq))
        .select(events::actor)
        .first::<Option<Uuid>>(conn)
        .optional()?
        .flatten())
}

pub fn github_url(app: &App, full_name: &str) -> String {
    format!("{}/{full_name}.git", app.config.github_git_base)
}

/// git config through the environment: the token never appears in argv.
pub(crate) fn auth_env(token: &str) -> Env {
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    vec![
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), "http.extraHeader".into()),
        (
            "GIT_CONFIG_VALUE_0".into(),
            format!("Authorization: Basic {basic}"),
        ),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
    ]
}

pub(crate) fn git_cmd(dir: &std::path::Path, env: &Env) -> std::process::Command {
    let mut c = std::process::Command::new("git");
    c.arg("-C")
        .arg(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .envs(env.iter().cloned());
    c
}

/// Refs on GitHub (heads and tags) as (name, sha).
pub fn ls_remote(
    dir: &std::path::Path,
    url: &str,
    token: Option<&str>,
) -> Result<Vec<(String, String)>, Failure> {
    let env = token.map(auth_env).unwrap_or_default();
    let out = git_cmd(dir, &env)
        .args(["ls-remote", "--heads", "--tags", "--refs", url])
        .output()
        .map_err(|e| Failure::Transient(format!("spawn git: {e}")))?;
    if !out.status.success() {
        return Err(github::classify_git(
            &String::from_utf8_lossy(&out.stderr),
            &String::from_utf8_lossy(&out.stdout),
        ));
    }
    crate::github_health::record_success(crate::github_health::Lane::Git);
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (sha, name) = l.split_once('\t')?;
            Some((name.trim().to_string(), sha.trim().to_string()))
        })
        .collect())
}

/// Fetch one remote ref into a local ref of the cache dir.
pub fn fetch_ref(
    dir: &std::path::Path,
    url: &str,
    token: Option<&str>,
    remote_ref: &str,
    local_ref: &str,
) -> Result<(), Failure> {
    let env = token.map(auth_env).unwrap_or_default();
    let out = git_cmd(dir, &env)
        .args([
            "fetch",
            "--quiet",
            "--no-tags",
            url,
            &format!("+{remote_ref}:{local_ref}"),
        ])
        .output()
        .map_err(|e| Failure::Transient(format!("spawn git: {e}")))?;
    if !out.status.success() {
        return Err(github::classify_git(
            &String::from_utf8_lossy(&out.stderr),
            &String::from_utf8_lossy(&out.stdout),
        ));
    }
    crate::github_health::record_success(crate::github_health::Lane::Git);
    Ok(())
}

pub enum PushResult {
    Ok,
    /// The lease did not match: GitHub's ref moved since we last mirrored it.
    StaleLease,
    Failed(Failure),
}

/// Push one ref (or delete it) with a lease on the last mirrored sha.
pub fn push_ref(
    dir: &std::path::Path,
    url: &str,
    token: &str,
    r#ref: &str,
    target: Option<&str>,
    lease: Option<&str>,
    force: bool,
) -> PushResult {
    let env = auth_env(token);
    let mut c = git_cmd(dir, &env);
    c.args(["push", "--porcelain", "--no-verify"]);
    if force {
        c.arg("--force");
    } else {
        c.arg(format!(
            "--force-with-lease={}:{}",
            r#ref,
            lease.unwrap_or("")
        ));
    }
    c.arg(url);
    c.arg(format!("{}:{}", target.unwrap_or(""), r#ref));
    let out = match c.output() {
        Ok(o) => o,
        Err(e) => return PushResult::Failed(Failure::Transient(format!("spawn git: {e}"))),
    };
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        crate::github_health::record_success(crate::github_health::Lane::Git);
        return PushResult::Ok;
    }
    if stdout.contains("stale info") || stderr.contains("stale info") {
        crate::github_health::record_success(crate::github_health::Lane::Git);
        return PushResult::StaleLease;
    }
    PushResult::Failed(github::classify_git(&stderr, &stdout))
}

fn record_attempt(
    conn: &mut PgConnection,
    action_id: Uuid,
    ok: bool,
    http_status: Option<i32>,
    error: Option<&str>,
) -> Result<(), Error> {
    diesel::insert_into(mirror_attempts::table)
        .values((
            mirror_attempts::id.eq(vcsstar_db::new_id()),
            mirror_attempts::action_id.eq(action_id),
            mirror_attempts::finished_at.eq(Utc::now()),
            mirror_attempts::ok.eq(ok),
            mirror_attempts::http_status.eq(http_status),
            mirror_attempts::error.eq(error),
        ))
        .execute(conn)?;
    diesel::update(mirror_actions::table.find(action_id))
        .set((
            mirror_actions::attempts.eq(mirror_actions::attempts + 1),
            mirror_actions::updated_at.eq(Utc::now()),
        ))
        .execute(conn)?;
    Ok(())
}

pub(crate) fn mirror_event(
    conn: &mut PgConnection,
    org: &Org,
    repo: &Repo,
    kind: &str,
    object_key: &str,
    action_kind: &str,
    error: Option<String>,
) -> Result<(), Error> {
    outbox::emit(
        conn,
        Emit {
            lane: outbox::lane_for_repo(&org.name, &repo.name),
            kind,
            repo_id: Some(repo.id),
            org_id: Some(org.id),
            actor_id: None,
            actor: None,
            repo: Some(vcs::repo_ref(org, repo)),
            org: Some(org.name.clone()),
            subject: Some(object_key.to_string()),
            payload: EventPayload::Variant4(MirrorPayload {
                object_key: object_key.to_string(),
                error,
                action_kind: action_kind.to_string(),
            }),
        },
    )?;
    Ok(())
}

fn set_blocked(
    conn: &mut PgConnection,
    org: &Org,
    repo: &Repo,
    action: &ActionRow,
    reason: &str,
) -> Result<(), Error> {
    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(mirror_actions::table.find(action.id))
            .set((
                mirror_actions::state.eq("blocked"),
                mirror_actions::blocked_reason.eq(reason),
                mirror_actions::blocked_at.eq(Utc::now()),
                mirror_actions::updated_at.eq(Utc::now()),
            ))
            .execute(conn)?;
        diesel::update(mirror_state::table.find(repo.id))
            .set(mirror_state::last_error.eq(reason))
            .execute(conn)?;
        mirror_event(
            conn,
            org,
            repo,
            "mirror_blocked",
            &action.object_key,
            &action.action_kind,
            Some(reason.to_string()),
        )
    })
}

fn set_applied(conn: &mut PgConnection, repo_id: Uuid, action: &ActionRow) -> Result<(), Error> {
    diesel::update(mirror_actions::table.find(action.id))
        .set((
            mirror_actions::state.eq("applied"),
            mirror_actions::updated_at.eq(Utc::now()),
        ))
        .execute(conn)?;
    diesel::update(mirror_state::table.find(repo_id))
        .set((
            mirror_state::last_success_at.eq(Utc::now()),
            mirror_state::last_error.eq::<Option<String>>(None),
        ))
        .execute(conn)?;
    Ok(())
}

pub(crate) fn set_mirrored(
    conn: &mut PgConnection,
    repo_id: Uuid,
    r#ref: &str,
    sha: Option<&str>,
) -> Result<(), Error> {
    match sha {
        Some(s) => {
            diesel::insert_into(mirror_refs::table)
                .values((
                    mirror_refs::repo_id.eq(repo_id),
                    mirror_refs::ref_.eq(r#ref),
                    mirror_refs::mirrored_sha.eq(s),
                    mirror_refs::last_pushed_at.eq(Utc::now()),
                    mirror_refs::last_verified_at.eq(Utc::now()),
                ))
                .on_conflict((mirror_refs::repo_id, mirror_refs::ref_))
                .do_update()
                .set((
                    mirror_refs::mirrored_sha.eq(s),
                    mirror_refs::last_pushed_at.eq(Utc::now()),
                    mirror_refs::last_verified_at.eq(Utc::now()),
                ))
                .execute(conn)?;
        }
        None => {
            diesel::delete(
                mirror_refs::table
                    .filter(mirror_refs::repo_id.eq(repo_id))
                    .filter(mirror_refs::ref_.eq(r#ref)),
            )
            .execute(conn)?;
        }
    }
    Ok(())
}

pub fn mirrored_sha(
    conn: &mut PgConnection,
    repo_id: Uuid,
    r#ref: &str,
) -> Result<Option<String>, Error> {
    Ok(mirror_refs::table
        .filter(mirror_refs::repo_id.eq(repo_id))
        .filter(mirror_refs::ref_.eq(r#ref))
        .select(mirror_refs::mirrored_sha)
        .first(conn)
        .optional()?)
}

/// Converge one action. Idempotent: it reads the desired state from
/// Postgres now and does nothing when GitHub already matches.
pub fn execute(app: &App, conn: &mut DbConn, action_id: Uuid) -> Result<Exec, Error> {
    let action: ActionRow = mirror_actions::table
        .find(action_id)
        .select(ActionRow::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("mirror action"))?;
    if action.state != "pending" {
        return Ok(Exec::Skipped(format!("action is {}", action.state)));
    }
    let (org, repo) = load_repo(conn, action.repo_id)?;
    let Some(state) = state_for(conn, repo.id)? else {
        return Ok(Exec::Skipped("mirror not configured".into()));
    };
    if state.paused || !repo.mirror_enabled {
        return Ok(Exec::Skipped("mirror paused".into()));
    }
    let payload = decode_payload(&action.payload_cbor)
        .ok_or_else(|| Error::internal("undecodable action payload"))?;
    let kind = payload.kind();
    let lane = match &payload {
        ActionPayload::Push { .. } => crate::github_health::Lane::Git,
        _ => crate::github_health::Lane::Api,
    };
    if let Some(wait) = crate::github_health::hold(lane) {
        crate::metrics::mirror_action(kind, "held");
        return Ok(Exec::Held(wait));
    }
    let outcome = match &payload {
        ActionPayload::Push { r#ref } => exec_push(app, conn, &org, &repo, &state, &action, r#ref),
        ActionPayload::RepoSettings => exec_settings(app, conn, &org, &repo, &state, &action),
        ActionPayload::PrSync { number } => {
            crate::github_sync::exec_pr_sync(app, conn, &org, &repo, &state, &action, *number)
        }
        ActionPayload::PrComment { id } => {
            crate::github_sync::exec_pr_comment(app, conn, &org, &repo, &state, &action, *id)
        }
        ActionPayload::PrReview { id } => {
            crate::github_sync::exec_pr_review(app, conn, &org, &repo, &state, &action, *id)
        }
        ActionPayload::Status { sha, context } => {
            crate::github_sync::exec_status(app, conn, &org, &repo, &state, &action, sha, context)
        }
    };
    crate::metrics::mirror_action(
        kind,
        match &outcome {
            Ok(Exec::Applied) => "applied",
            Ok(Exec::Skipped(_)) => "skipped",
            Ok(Exec::Blocked(_)) => "blocked",
            Ok(Exec::Transient(_)) => "transient",
            Ok(Exec::Held(_)) => "held",
            Err(_) => "error",
        },
    );
    match outcome {
        Ok(Exec::Applied) => {
            record_attempt(conn, action.id, true, None, None)?;
            set_applied(conn, repo.id, &action)?;
            Ok(Exec::Applied)
        }
        Ok(Exec::Blocked(reason)) => {
            record_attempt(conn, action.id, false, None, Some(&reason))?;
            set_blocked(conn, &org, &repo, &action, &reason)?;
            cred::forget_app_token(app, conn, org.id, repo.id, &action.credential_purpose);
            Ok(Exec::Blocked(reason))
        }
        Ok(Exec::Transient(reason)) => {
            record_attempt(conn, action.id, false, None, Some(&reason))?;
            diesel::update(mirror_state::table.find(repo.id))
                .set(mirror_state::last_error.eq(&reason))
                .execute(conn)?;
            Ok(Exec::Transient(reason))
        }
        Ok(other) => Ok(other),
        Err(e) => Err(e),
    }
}

fn exec_push(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    r#ref: &str,
) -> Result<Exec, Error> {
    let target = refs::get(conn, repo.id, r#ref)?;
    let lease = mirrored_sha(conn, repo.id, r#ref)?;
    if target.as_ref().map(|t| t.0.as_str()) == lease.as_deref() {
        return Ok(Exec::Applied); // already converged
    }
    let dir = vcs::hydrate(app, conn, repo)?;
    if let Some(t) = &target {
        if !app.store.backend().object_exists(&dir, t, &Env::new()) {
            return Ok(Exec::Transient(format!(
                "object {t} is not in this node's cache yet"
            )));
        }
    }
    let actor = event_actor(conn, action)?;
    let used = match cred::token_for_actor(
        app,
        conn,
        org.id,
        repo.id,
        &action.credential_purpose,
        actor,
    ) {
        Ok(v) => v,
        Err(Failure::Terminal(m)) => return Ok(Exec::Blocked(m)),
        Err(Failure::Transient(m)) => return Ok(Exec::Transient(m)),
    };
    tracing::info!(action = %action.object_key, source = %used.label(), "GitHub credential chosen");
    let token = used.value.clone();
    let url = github_url(app, &state.github_repo);
    let target_str = target.as_ref().map(|t| t.0.as_str());
    if target.is_none() && lease.is_some() {
        if let Some(branch) = r#ref.strip_prefix("refs/heads/") {
            let gh = github::Client::new(&app.config.github_api_base, &token);
            match crate::github_sync::prepare_branch_delete(conn, repo, state, &gh, branch) {
                Ok(n) if n > 0 => {
                    tracing::info!(repo = %state.github_repo, branch, moved = n, "retargeted GitHub pull requests before the branch delete")
                }
                Ok(_) => {}
                Err(Failure::Terminal(m)) => return Ok(Exec::Blocked(m)),
                Err(Failure::Transient(m)) => return Ok(Exec::Transient(m)),
            }
        }
    }
    match push_ref(
        &dir,
        &url,
        &token,
        r#ref,
        target_str,
        lease.as_deref(),
        false,
    ) {
        PushResult::Ok => {
            set_mirrored(conn, repo.id, r#ref, target_str)?;
            Ok(Exec::Applied)
        }
        PushResult::StaleLease => handle_drift(
            app,
            conn,
            org,
            repo,
            state,
            action,
            &dir,
            &url,
            &token,
            r#ref,
            target_str,
            lease.as_deref(),
        ),
        PushResult::Failed(Failure::Terminal(m)) => Ok(crate::github_sync::settle_refusal(
            conn,
            &used,
            Exec::Blocked(m),
        )),
        PushResult::Failed(Failure::Transient(m)) => Ok(Exec::Transient(m)),
    }
}

/// GitHub's ref moved behind our back. Preserve what is there, then act
/// by mode.
#[allow(clippy::too_many_arguments)]
fn handle_drift(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    _action: &ActionRow,
    dir: &std::path::Path,
    url: &str,
    token: &str,
    r#ref: &str,
    target: Option<&str>,
    lease: Option<&str>,
) -> Result<Exec, Error> {
    let remote = ls_remote(dir, url, Some(token));
    let remote_sha = match remote {
        Ok(list) => list.into_iter().find(|(n, _)| n == r#ref).map(|(_, s)| s),
        Err(Failure::Transient(m)) => return Ok(Exec::Transient(m)),
        Err(Failure::Terminal(m)) => return Ok(Exec::Blocked(m)),
    };
    let Some(remote_sha) = remote_sha else {
        // The ref is gone on GitHub: lease mismatch because it was deleted.
        return match push_ref(dir, url, token, r#ref, target, None, true) {
            PushResult::Ok => {
                set_mirrored(conn, repo.id, r#ref, target)?;
                mirror_event(
                    conn,
                    org,
                    repo,
                    "mirror_drift",
                    r#ref,
                    "git_push",
                    Some(format!(
                        "{ref} was deleted on GitHub (expected {}); re-pushed",
                        lease.unwrap_or("<absent>")
                    )),
                )?;
                Ok(Exec::Applied)
            }
            PushResult::StaleLease => Ok(Exec::Transient("lease raced again".into())),
            PushResult::Failed(Failure::Terminal(m)) => Ok(Exec::Blocked(m)),
            PushResult::Failed(Failure::Transient(m)) => Ok(Exec::Transient(m)),
        };
    };
    // Keep the drifted commits: fetch into an internal ref and record it.
    let short = r#ref.trim_start_matches("refs/");
    let drift_ref = format!(
        "refs/vcsstar/drift/{}/{short}",
        Utc::now().format("%Y%m%dT%H%M%S")
    );
    match fetch_ref(dir, url, Some(token), r#ref, &drift_ref) {
        Ok(()) => {
            let _ = refs::apply(
                conn,
                repo.id,
                &[RefUpdate {
                    name: drift_ref.clone(),
                    old: None,
                    new: Some(RevId(remote_sha.clone())),
                }],
                None,
            );
        }
        Err(Failure::Transient(m)) => return Ok(Exec::Transient(m)),
        Err(Failure::Terminal(m)) => return Ok(Exec::Blocked(m)),
    }
    let note = format!(
        "GitHub {ref} is at {remote_sha}, expected {}; drifted commits kept at {drift_ref}",
        lease.unwrap_or("<absent>")
    );
    match state.mode.as_str() {
        "authoritative_vcsstar" => {
            match push_ref(dir, url, token, r#ref, target, Some(&remote_sha), false) {
                PushResult::Ok => {
                    set_mirrored(conn, repo.id, r#ref, target)?;
                    mirror_event(
                        conn,
                        org,
                        repo,
                        "mirror_drift",
                        r#ref,
                        "git_push",
                        Some(format!("{note}; overwritten")),
                    )?;
                    Ok(Exec::Applied)
                }
                PushResult::StaleLease => Ok(Exec::Transient("lease raced again".into())),
                PushResult::Failed(Failure::Terminal(m)) => Ok(Exec::Blocked(m)),
                PushResult::Failed(Failure::Transient(m)) => Ok(Exec::Transient(m)),
            }
        }
        "import_drift" => {
            // Fast-forward drift becomes a bot push into vcsstar.
            let backend = app.store.backend();
            let ours = target.map(|t| RevId(t.to_string()));
            let ff = match &ours {
                Some(o) => backend
                    .is_ancestor(dir, o, &RevId(remote_sha.clone()), &Env::new())
                    .unwrap_or(false),
                None => true,
            };
            if ff {
                let update = RefUpdate {
                    name: r#ref.to_string(),
                    old: ours.clone(),
                    new: Some(RevId(remote_sha.clone())),
                };
                let commits = crate::services::vcs_ops::summaries(
                    app,
                    dir,
                    ours.as_ref(),
                    &RevId(remote_sha.clone()),
                )?;
                conn.transaction::<(), Error, _>(|conn| {
                    push::apply_ref_change(conn, org, repo, None, &update, false, commits)?;
                    mirror_event(
                        conn,
                        org,
                        repo,
                        "mirror_drift",
                        r#ref,
                        "git_push",
                        Some(format!("{note}; imported")),
                    )
                })?;
                set_mirrored(conn, repo.id, r#ref, Some(&remote_sha))?;
                crate::services::prs::on_push(
                    app,
                    conn,
                    org,
                    repo,
                    None,
                    std::slice::from_ref(&update),
                )?;
                Ok(Exec::Applied)
            } else {
                mirror_event(
                    conn,
                    org,
                    repo,
                    "mirror_drift",
                    r#ref,
                    "git_push",
                    Some(note.clone()),
                )?;
                Ok(Exec::Blocked(format!("diverged: {note}")))
            }
        }
        _ => {
            mirror_event(
                conn,
                org,
                repo,
                "mirror_drift",
                r#ref,
                "git_push",
                Some(note.clone()),
            )?;
            Ok(Exec::Blocked(format!("drift flagged: {note}")))
        }
    }
}

/// GitHub's ref is observed at `observed` while we last mirrored `lease`.
/// Keep the drifted commits under `refs/vcsstar/drift/<ts>/<ref>`, emit
/// `mirror_drift`, then act by mode. Returns true when a push action
/// should converge the ref (the recorded lease now equals `observed`, so
/// the executor's lease matches and overwrites in one step).
#[allow(clippy::too_many_arguments)]
pub fn note_drift(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    dir: &std::path::Path,
    token: Option<&str>,
    r#ref: &str,
    observed: Option<&str>,
) -> Result<bool, Error> {
    let lease = mirrored_sha(conn, repo.id, r#ref)?;
    if lease.as_deref() == observed {
        return Ok(false);
    }
    let url = github_url(app, &state.github_repo);
    let mut kept = None;
    if let Some(sha) = observed {
        let short = r#ref.trim_start_matches("refs/");
        let drift_ref = format!(
            "refs/vcsstar/drift/{}/{short}",
            Utc::now().format("%Y%m%dT%H%M%S")
        );
        match fetch_ref(dir, &url, token, r#ref, &drift_ref) {
            Ok(()) => {
                let _ = refs::apply(
                    conn,
                    repo.id,
                    &[RefUpdate {
                        name: drift_ref.clone(),
                        old: None,
                        new: Some(RevId(sha.to_string())),
                    }],
                    None,
                );
                kept = Some(drift_ref);
            }
            Err(f) => return Err(f.into()),
        }
    }
    let note = format!(
        "GitHub {ref} is at {}, expected {}{}",
        observed.unwrap_or("<absent>"),
        lease.as_deref().unwrap_or("<absent>"),
        kept.as_ref()
            .map(|k| format!("; drifted commits kept at {k}"))
            .unwrap_or_default()
    );
    // The lease now reflects what GitHub holds.
    set_mirrored(conn, repo.id, r#ref, observed)?;
    match state.mode.as_str() {
        "authoritative_vcsstar" => {
            mirror_event(
                conn,
                org,
                repo,
                "mirror_drift",
                r#ref,
                "git_push",
                Some(format!("{note}; scheduling overwrite")),
            )?;
            Ok(true)
        }
        "import_drift" => {
            let ours = refs::get(conn, repo.id, r#ref)?;
            let backend = app.store.backend();
            match observed {
                Some(sha) => {
                    let ff = match &ours {
                        Some(o) => backend
                            .is_ancestor(dir, o, &RevId(sha.to_string()), &Env::new())
                            .unwrap_or(false),
                        None => true,
                    };
                    if ff {
                        let update = RefUpdate {
                            name: r#ref.to_string(),
                            old: ours.clone(),
                            new: Some(RevId(sha.to_string())),
                        };
                        let commits = crate::services::vcs_ops::summaries(
                            app,
                            dir,
                            ours.as_ref(),
                            &RevId(sha.to_string()),
                        )?;
                        conn.transaction::<(), Error, _>(|conn| {
                            push::apply_ref_change(conn, org, repo, None, &update, false, commits)?;
                            mirror_event(
                                conn,
                                org,
                                repo,
                                "mirror_drift",
                                r#ref,
                                "git_push",
                                Some(format!("{note}; imported")),
                            )
                        })?;
                        crate::services::prs::on_push(
                            app,
                            conn,
                            org,
                            repo,
                            None,
                            std::slice::from_ref(&update),
                        )?;
                        Ok(false)
                    } else {
                        mirror_event(
                            conn,
                            org,
                            repo,
                            "mirror_drift",
                            r#ref,
                            "git_push",
                            Some(format!("{note}; diverged, flagged")),
                        )?;
                        Ok(false)
                    }
                }
                None => {
                    // Deleted on GitHub. When vcsstar did not move the ref
                    // since it was mirrored, the deletion is imported (a
                    // merged PR branch cleaned up there); otherwise vcsstar's
                    // newer state is pushed back.
                    let ours = refs::get(conn, repo.id, r#ref)?;
                    let unchanged = ours.as_ref().map(|r| r.0.as_str()) == lease.as_deref();
                    if let (Some(current), true) = (ours.clone(), unchanged) {
                        let update = RefUpdate {
                            name: r#ref.to_string(),
                            old: Some(current),
                            new: None,
                        };
                        conn.transaction::<(), Error, _>(|conn| {
                            push::apply_ref_change(conn, org, repo, None, &update, false, vec![])?;
                            mirror_event(
                                conn,
                                org,
                                repo,
                                "mirror_drift",
                                r#ref,
                                "git_push",
                                Some(format!("{note}; deletion imported")),
                            )
                        })?;
                        crate::services::prs::on_push(
                            app,
                            conn,
                            org,
                            repo,
                            None,
                            std::slice::from_ref(&update),
                        )?;
                        return Ok(false);
                    }
                    mirror_event(
                        conn,
                        org,
                        repo,
                        "mirror_drift",
                        r#ref,
                        "git_push",
                        Some(format!("{note}; deleted on GitHub, scheduling re-push")),
                    )?;
                    Ok(true)
                }
            }
        }
        _ => {
            mirror_event(
                conn,
                org,
                repo,
                "mirror_drift",
                r#ref,
                "git_push",
                Some(format!("{note}; flagged")),
            )?;
            Ok(false)
        }
    }
}

fn exec_settings(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
) -> Result<Exec, Error> {
    let (_p, token) = match cred::token_for(app, conn, org.id, repo.id, &action.credential_purpose)
    {
        Ok(v) => v,
        Err(Failure::Terminal(m)) => return Ok(Exec::Blocked(m)),
        Err(Failure::Transient(m)) => return Ok(Exec::Transient(m)),
    };
    let gh = github::Client::new(&app.config.github_api_base, &token);
    let (owner, name) = state
        .github_repo
        .split_once('/')
        .ok_or_else(|| Error::invalid("github_repo must be owner/name"))?;
    let private = repo.visibility != "public";
    let existing = match gh.get_repo(&state.github_repo) {
        Ok(v) => v,
        Err(Failure::Terminal(m)) => return Ok(Exec::Blocked(m)),
        Err(Failure::Transient(m)) => return Ok(Exec::Transient(m)),
    };
    if existing.is_none() {
        if let Err(f) = gh.create_repo(owner, name, repo.description.as_deref(), private) {
            return Ok(match f {
                Failure::Terminal(m) => Exec::Blocked(m),
                Failure::Transient(m) => Exec::Transient(m),
            });
        }
    }
    let default_ref = format!("refs/heads/{}", repo.default_branch);
    let default_mirrored = mirrored_sha(conn, repo.id, &default_ref)?.is_some();
    let mut patch = serde_json::json!({
        "description": repo.description.clone().unwrap_or_default(),
        "private": private,
        "archived": repo.archived,
    });
    if default_mirrored {
        patch["default_branch"] = serde_json::json!(repo.default_branch);
    }
    match gh.patch_repo(&state.github_repo, &patch) {
        Ok(_) => Ok(Exec::Applied),
        Err(Failure::Terminal(m)) => Ok(Exec::Blocked(m)),
        Err(Failure::Transient(m)) => Ok(Exec::Transient(m)),
    }
}

// ---------------------------------------------------------------------
// Reconcile: compare GitHub with Postgres, schedule convergence
// ---------------------------------------------------------------------

pub struct ReconcileOutcome {
    pub refs_checked: usize,
    pub scheduled: Vec<Uuid>,
    pub drift: Vec<String>,
}

/// State-based comparison. Creates pending actions for refs that differ
/// (and a delete for GitHub-only refs in authoritative mode); stamps
/// verified refs. The caller submits tasks for `scheduled`.
pub fn reconcile(app: &App, conn: &mut DbConn, repo_id: Uuid) -> Result<ReconcileOutcome, Error> {
    let (org, repo) = load_repo(conn, repo_id)?;
    let state = state_for(conn, repo_id)?
        .ok_or_else(|| Error::new(Code::PreconditionFailed, "mirror not configured"))?;
    let dir = vcs::hydrate(app, conn, &repo)?;
    let token = cred::token_for(app, conn, org.id, repo.id, "mirror_git_push")
        .ok()
        .map(|(_, t)| t);
    let remote = ls_remote(&dir, &github_url(app, &state.github_repo), token.as_deref())?;
    let ours: Vec<(String, RevId)> = refs::load(conn, repo_id)?
        .into_iter()
        .filter(|(n, _)| n.starts_with("refs/heads/") || n.starts_with("refs/tags/"))
        .collect();
    let remote_map: std::collections::HashMap<&str, &str> = remote
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let mut out = ReconcileOutcome {
        refs_checked: 0,
        scheduled: vec![],
        drift: vec![],
    };
    let now = Utc::now();
    let lane = outbox::lane_for_repo(&org.name, &repo.name);
    let cursor = state.cursor_seq;
    for (name, target) in &ours {
        out.refs_checked += 1;
        match remote_map.get(name.as_str()) {
            Some(sha) if *sha == target.0 => {
                set_mirrored(conn, repo_id, name, Some(&target.0))?;
            }
            other => {
                let observed = other.copied();
                out.drift.push(format!(
                    "{name}: GitHub {}, vcsstar {target}",
                    observed.unwrap_or("<absent>")
                ));
                let push = note_drift(
                    app,
                    conn,
                    &org,
                    &repo,
                    &state,
                    &dir,
                    token.as_deref(),
                    name,
                    observed,
                )?;
                if push || observed.is_none() {
                    schedule(conn, &lane, cursor, repo_id, name, &mut out)?;
                }
            }
        }
    }
    for (name, sha) in &remote {
        if !ours.iter().any(|(n, _)| n == name) {
            out.refs_checked += 1;
            out.drift.push(format!("{name}: only on GitHub ({sha})"));
            let push = note_drift(
                app,
                conn,
                &org,
                &repo,
                &state,
                &dir,
                token.as_deref(),
                name,
                Some(sha),
            )?;
            if push {
                // Desired state is "absent": the push action deletes it.
                schedule(conn, &lane, cursor, repo_id, name, &mut out)?;
            }
        }
    }
    diesel::update(mirror_state::table.find(repo_id))
        .set(mirror_state::last_full_reconcile_at.eq(now))
        .execute(conn)?;
    Ok(out)
}

fn schedule(
    conn: &mut PgConnection,
    lane: &str,
    seq: i64,
    repo_id: Uuid,
    r#ref: &str,
    out: &mut ReconcileOutcome,
) -> Result<(), Error> {
    let spec = ActionSpec {
        kind: "git_push",
        object_key: r#ref.to_string(),
        purpose: "mirror_git_push",
        payload: ActionPayload::Push {
            r#ref: r#ref.to_string(),
        },
    };
    let ev = EventRef {
        repo_id,
        lane,
        seq,
        kind: "reconcile",
        payload_cbor: &[],
    };
    let id = upsert_action(conn, &ev, &spec)?;
    out.scheduled.push(id);
    Ok(())
}

/// Submit a wake-up task for a pending action.
pub fn submit_action(
    app: &App,
    org: &str,
    repo: &str,
    action_id: Uuid,
    seq: i64,
) -> Result<(), Error> {
    app.submit_task(
        &queue_name(org, repo),
        &format!("mirror:{action_id}:{seq}"),
        encode_task(&MirrorTask {
            action_id: action_id.to_string(),
        }),
        0,
    )
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MirrorTask {
    pub action_id: String,
}

pub fn encode_task(t: &MirrorTask) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(t, &mut out).expect("cbor");
    out
}

pub fn decode_task(b: &[u8]) -> Option<MirrorTask> {
    ciborium::de::from_reader(b).ok()
}

/// Backoff for transient mirror failures: 30 s doubling to 15 min.
pub fn backoff(attempts: i32) -> std::time::Duration {
    let secs = 30u64.saturating_mul(1u64 << (attempts.clamp(1, 10) - 1));
    std::time::Duration::from_secs(secs.min(900))
}
