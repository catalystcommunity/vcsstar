//! Both-ways sync of pull requests, reviews, comments, and commit statuses
//! between a mirrored repository and its GitHub repository.
//!
//! Outbound (vcsstar -> GitHub): the dispatcher turns PR and status events
//! into mirror actions (`mirror::expand_event`), one per object key; the
//! executors here converge one GitHub object each. Inbound (GitHub ->
//! vcsstar): the ingest worker hands a webhook payload to `ingest`, which
//! upserts the vcsstar rows and emits the usual PR events with a *shadow
//! identity* as actor. A shadow identity (`<uuid v5>@github.com`) stands
//! for one GitHub account; it never logs in, and the dispatcher never sends
//! the events it causes back to GitHub.
//!
//! Echo protection has two layers. Every synced row remembers the GitHub
//! id it is linked to, so a webhook about an object vcsstar created is
//! matched, not duplicated. Bodies vcsstar posts also carry an HTML
//! comment marker that names the vcsstar row, which covers the window
//! between the POST and the stored id.
//!
//! What stays on one side: a pull request whose head lives in a fork is
//! imported into vcsstar but a vcsstar fork PR is not created on GitHub;
//! a squash or rebase merge made on vcsstar closes the GitHub pull request
//! with a comment, because GitHub only detects a merge whose commits are
//! reachable from the base.

use chrono::Utc;
use diesel::prelude::*;
use serde_json::{json, Value as Json};
use uuid::Uuid;
use vcsstar_db::models::{Identity, NewIdentity, Org, Repo};
use vcsstar_db::schema::{
    github_identities, identities, pr_comments, pr_counters, pr_reviews, pull_requests, statuses,
};
use vcsstar_db::DbConn;
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_types::{EventPayload, StatusPayload};
use vcsstar_vcs::{Env, RevId};

use crate::github::{self, Failure};
use crate::github_link;
use crate::mirror::cred::{Token, TokenSource};
use crate::mirror::{self, ActionRow, Exec, StateRow};
use crate::outbox::{self, Emit};
use crate::principal::{CredentialKind, Principal};
use crate::services::prs::{self, CommentRow, PrRow, ReviewRow};
use crate::services::statuses::StatusRow;
use crate::{push, vcs, App, Code, Error};

/// The identity domain of GitHub shadow identities.
pub const SHADOW_DOMAIN: &str = "github.com";

// ---------------------------------------------------------------------
// Shadow identities
// ---------------------------------------------------------------------

/// The stable `user_id` of a GitHub account's shadow identity.
pub fn shadow_user_id(github_id: i64) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("https://api.github.com/user/{github_id}").as_bytes(),
    )
}

/// The shadow identity of a GitHub `user` object (`{id, login}`), made on
/// first sight and renamed when the login changed.
pub fn shadow_identity(conn: &mut PgConnection, user: &Json) -> Result<Identity, Error> {
    let github_id = user
        .get("id")
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("GitHub user object without id"))?;
    let login = user
        .get("login")
        .and_then(Json::as_str)
        .unwrap_or("github-user");
    let user_id = shadow_user_id(github_id);
    let existing: Option<Identity> = identities::table
        .filter(identities::lk_user_id.eq(user_id))
        .filter(identities::lk_domain.eq(SHADOW_DOMAIN))
        .select(Identity::as_select())
        .first(conn)
        .optional()?;
    let identity = match existing {
        Some(i) if i.handle == login => i,
        Some(i) => diesel::update(identities::table.find(i.id))
            .set((
                identities::handle.eq(login),
                identities::display_name.eq(login),
            ))
            .returning(Identity::as_returning())
            .get_result(conn)?,
        None => diesel::insert_into(identities::table)
            .values(&NewIdentity {
                id: vcsstar_db::new_id(),
                lk_user_id: user_id,
                lk_domain: SHADOW_DOMAIN,
                handle: login,
                display_name: Some(login),
                email: None,
                email_verified: false,
            })
            .returning(Identity::as_returning())
            .get_result(conn)?,
    };
    diesel::insert_into(github_identities::table)
        .values((
            github_identities::identity_id.eq(identity.id),
            github_identities::github_id.eq(github_id),
            github_identities::login.eq(login),
        ))
        .on_conflict(github_identities::identity_id)
        .do_update()
        .set((
            github_identities::login.eq(login),
            github_identities::updated_at.eq(Utc::now()),
        ))
        .execute(conn)?;
    Ok(identity)
}

/// Whether an event actor is a shadow identity, i.e. GitHub caused the
/// change and must not receive it again.
pub fn is_shadow_actor(conn: &mut PgConnection, actor: Option<Uuid>) -> Result<bool, Error> {
    let Some(id) = actor else {
        return Ok(false);
    };
    let domain: Option<String> = identities::table
        .find(id)
        .select(identities::lk_domain)
        .first(conn)
        .optional()?;
    Ok(domain.as_deref() == Some(SHADOW_DOMAIN))
}

/// A principal for an identity acting from GitHub (a shadow, or a linked
/// vcsstar identity): no scopes, only used as an event actor and as the
/// ref-update actor of imports.
pub fn shadow_principal(identity: Identity) -> Principal {
    Principal {
        identity,
        scopes: Vec::new(),
        kind: CredentialKind::ApiToken,
        acting_via: Some("github".into()),
        credential_id: None,
        expires_at: None,
        instance_role: None,
    }
}

// ---------------------------------------------------------------------
// Markers and bodies
// ---------------------------------------------------------------------

fn marker(kind: &str, id: &str) -> String {
    format!("<!-- vcsstar:{kind}:{id} -->")
}

/// The id named by a `<!-- vcsstar:<kind>:<id> -->` marker in a body.
pub fn find_marker(body: &str, kind: &str) -> Option<String> {
    let prefix = format!("<!-- vcsstar:{kind}:");
    let start = body.find(&prefix)? + prefix.len();
    let rest = &body[start..];
    let end = rest.find(" -->")?;
    Some(rest[..end].trim().to_string())
}

/// `\r\n` to `\n`, trailing whitespace dropped: GitHub stores what the
/// browser sent, the API what we sent, and the two must compare equal.
pub fn normalize(body: &str) -> String {
    body.replace("\r\n", "\n").trim_end().to_string()
}

const FOOTER_START: &str = "\n\n---\n_Synced from [vcsstar pull request";

/// A GitHub body without the footer and markers vcsstar appends.
pub fn strip_vcsstar(body: &str) -> String {
    let body = normalize(body);
    let cut = body.find(FOOTER_START).map(|i| &body[..i]).unwrap_or(&body);
    cut.lines()
        .filter(|l| !l.trim_start().starts_with("<!-- vcsstar:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn display_name(i: &vcsstar_types::Identity) -> String {
    let handle = i.handle.clone().unwrap_or_else(|| i.user_id.clone());
    match &i.display_name {
        Some(d) if !d.is_empty() && *d != handle => format!("{d} ({handle}@{})", i.domain),
        _ => format!("{handle}@{}", i.domain),
    }
}

fn pr_url(app: &App, org: &Org, repo: &Repo, number: i32) -> String {
    format!(
        "{}/{}/{}/pulls/{number}",
        app.config.public_url.trim_end_matches('/'),
        org.name,
        repo.name
    )
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

// ---------------------------------------------------------------------
// Inbound: webhook payloads -> vcsstar rows
// ---------------------------------------------------------------------

/// Apply one webhook delivery about `state.github_repo`. Returns a short
/// note for the log.
pub fn ingest(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    event: &str,
    body: &Json,
) -> Result<String, Error> {
    match event {
        "pull_request" => ingest_pull_request(app, conn, org, repo, state, body),
        "pull_request_review" => ingest_review(app, conn, org, repo, state, body),
        "pull_request_review_comment" => ingest_review_comment(app, conn, org, repo, state, body),
        "issue_comment" => ingest_issue_comment(app, conn, org, repo, state, body),
        "status" => ingest_status(conn, org, repo, body),
        "check_run" => ingest_check_run(conn, org, repo, body),
        other => Ok(format!("ignored event {other}")),
    }
}

/// The fields of a GitHub pull request object the sync reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhPull {
    pub number: i64,
    pub title: String,
    pub body: String,
    /// `open` | `closed` | `merged` (GitHub says closed + merged).
    pub state: &'static str,
    pub merge_commit_sha: Option<String>,
    pub merged_by: Option<Json>,
    pub user: Json,
    pub head_sha: String,
    pub head_branch: String,
    pub head_repo: Option<String>,
    pub base_sha: String,
    pub base_branch: String,
}

pub fn parse_pull(j: &Json) -> Result<GhPull, Error> {
    let s = |p: &str| j.pointer(p).and_then(Json::as_str).map(str::to_string);
    let number = j
        .get("number")
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("pull_request without number"))?;
    let merged = j.get("merged").and_then(Json::as_bool).unwrap_or(false)
        || j.get("merged_at").and_then(Json::as_str).is_some();
    let state = match s("/state").as_deref() {
        Some("open") => "open",
        _ if merged => "merged",
        _ => "closed",
    };
    Ok(GhPull {
        number,
        title: s("/title").unwrap_or_default(),
        body: strip_vcsstar(&s("/body").unwrap_or_default()),
        state,
        merge_commit_sha: s("/merge_commit_sha"),
        merged_by: j.get("merged_by").filter(|v| v.is_object()).cloned(),
        user: j.get("user").cloned().unwrap_or(Json::Null),
        head_sha: s("/head/sha").ok_or_else(|| Error::invalid("pull_request without head.sha"))?,
        head_branch: s("/head/ref")
            .ok_or_else(|| Error::invalid("pull_request without head.ref"))?,
        head_repo: s("/head/repo/full_name"),
        base_sha: s("/base/sha").unwrap_or_default(),
        base_branch: s("/base/ref")
            .ok_or_else(|| Error::invalid("pull_request without base.ref"))?,
    })
}

/// The identity behind the webhook's `sender`: the linked vcsstar
/// identity when the GitHub account is linked, else its shadow.
fn sender(conn: &mut PgConnection, body: &Json) -> Result<Identity, Error> {
    github_link::identity_for_user(conn, body.get("sender").unwrap_or(&Json::Null))
}

fn pr_by_github_number(
    conn: &mut PgConnection,
    repo_id: Uuid,
    number: i64,
) -> Result<Option<PrRow>, Error> {
    Ok(pull_requests::table
        .filter(pull_requests::repo_id.eq(repo_id))
        .filter(pull_requests::github_number.eq(number.min(i32::MAX as i64) as i32))
        .select(PrRow::as_select())
        .first(conn)
        .optional()?)
}

fn mirror_token(app: &App, conn: &mut PgConnection, org: &Org, repo: &Repo) -> Option<String> {
    mirror::cred::token_for(app, conn, org.id, repo.id, "import_fetch")
        .ok()
        .map(|(_, t)| t)
}

/// Make sure the head commit of GitHub PR `number` is in the cache
/// directory and recorded as a segment. `refs/pull/{n}/head` exists for
/// fork heads too, so one fetch covers both cases.
#[allow(clippy::too_many_arguments)]
fn ensure_pull_objects(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    dir: &std::path::Path,
    number: i64,
    head: &RevId,
) -> Result<(), Error> {
    let backend = app.store.backend();
    if backend.object_exists(dir, head, &Env::new()) {
        return Ok(());
    }
    let token = mirror_token(app, conn, org, repo);
    let url = mirror::github_url(app, &state.github_repo);
    let local = format!("refs/vcsstar/import/pull/{number}");
    mirror::fetch_ref(
        dir,
        &url,
        token.as_deref(),
        &format!("refs/pull/{number}/head"),
        &local,
    )?;
    if !backend.object_exists(dir, head, &Env::new()) {
        return Err(Error::new(
            Code::Unavailable,
            format!("GitHub refs/pull/{number}/head does not hold {head} yet"),
        ));
    }
    let have: Vec<RevId> = refs::load(conn, repo.id)?
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    vcs::capture(app, conn, repo.id, dir, &have, std::slice::from_ref(head))?;
    Ok(())
}

/// Bring a same-repository head branch into vcsstar when it is absent or
/// behind (a branch pushed to GitHub first). Only in `import_drift` mode:
/// in `authoritative_vcsstar` mode GitHub-only branches are drift that the
/// reconcile pass removes, and the PR keeps its snapshot regardless.
#[allow(clippy::too_many_arguments)]
fn import_head_branch(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    dir: &std::path::Path,
    actor: &Principal,
    head_ref: &str,
    head: &RevId,
) -> Result<(), Error> {
    if state.mode != "import_drift" {
        return Ok(());
    }
    let ours = refs::get(conn, repo.id, head_ref)?;
    if ours.as_ref() == Some(head) {
        return Ok(());
    }
    let backend = app.store.backend();
    let ff = match &ours {
        Some(o) => backend
            .is_ancestor(dir, o, head, &Env::new())
            .unwrap_or(false),
        None => true,
    };
    if !ff {
        return Ok(());
    }
    let update = RefUpdate {
        name: head_ref.to_string(),
        old: ours.clone(),
        new: Some(head.clone()),
    };
    let commits = crate::services::vcs_ops::summaries(app, dir, ours.as_ref(), head)?;
    conn.transaction::<(), Error, _>(|conn| {
        push::apply_ref_change(conn, org, repo, Some(actor), &update, false, commits)
    })?;
    mirror::set_mirrored(conn, repo.id, head_ref, Some(&head.0))?;
    prs::on_push(
        app,
        conn,
        org,
        repo,
        Some(actor),
        std::slice::from_ref(&update),
    )?;
    Ok(())
}

/// The vcsstar number of a pull request imported from GitHub: GitHub's
/// own number when it is free here (so "#123" in a commit message points
/// at the same pull request on both sides), else the next free one. The
/// counter moves past an adopted number, so later vcsstar pull requests
/// never take it.
fn number_for_import(
    conn: &mut PgConnection,
    repo_id: Uuid,
    github_number: i64,
) -> Result<i32, Error> {
    let Ok(n) = i32::try_from(github_number) else {
        return next_number(conn, repo_id);
    };
    if n < 1 || n == i32::MAX {
        return next_number(conn, repo_id);
    }
    let taken: bool = diesel::select(diesel::dsl::exists(
        pull_requests::table
            .filter(pull_requests::repo_id.eq(repo_id))
            .filter(pull_requests::number.eq(n)),
    ))
    .get_result(conn)?;
    if taken {
        return next_number(conn, repo_id);
    }
    diesel::sql_query(
        "INSERT INTO pr_counters (repo_id, next_number) VALUES ($1, $2) \
         ON CONFLICT (repo_id) DO UPDATE SET next_number = GREATEST(pr_counters.next_number, EXCLUDED.next_number)",
    )
    .bind::<diesel::sql_types::Uuid, _>(repo_id)
    .bind::<diesel::sql_types::Integer, _>(n + 1)
    .execute(conn)?;
    Ok(n)
}

fn next_number(conn: &mut PgConnection, repo_id: Uuid) -> Result<i32, Error> {
    Ok(diesel::insert_into(pr_counters::table)
        .values((
            pr_counters::repo_id.eq(repo_id),
            pr_counters::next_number.eq(2),
        ))
        .on_conflict(pr_counters::repo_id)
        .do_update()
        .set(pr_counters::next_number.eq(pr_counters::next_number + 1))
        .returning(pr_counters::next_number)
        .get_result::<i32>(conn)?
        - 1)
}

/// Create or update the vcsstar PR that shadows GitHub pull request `j`.
/// Returns the row and the event kinds emitted.
pub fn upsert_pull(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    j: &Json,
    actor: &Principal,
) -> Result<(PrRow, Vec<&'static str>), Error> {
    let gh = parse_pull(j)?;
    // GitHub's `updated_at` of this snapshot. A snapshot older than the
    // one last applied (a redelivery, or deliveries out of order) carries
    // nothing newer and is ignored.
    let incoming = rfc3339(j, "updated_at");
    let same_repo = gh
        .head_repo
        .as_deref()
        .map(|f| f.eq_ignore_ascii_case(&state.github_repo))
        .unwrap_or(false);
    let head_ref = format!("refs/heads/{}", gh.head_branch);
    let base_ref = format!("refs/heads/{}", gh.base_branch);
    let head = RevId(gh.head_sha.clone());
    let mut existing = pr_by_github_number(conn, repo.id, gh.number)?;
    if existing.is_none() {
        // A pull request vcsstar created: link it by the marker in its body
        // when the webhook outran the stored number.
        let raw = j.get("body").and_then(Json::as_str).unwrap_or("");
        if let Some(n) = find_marker(raw, "pr").and_then(|n| n.parse::<u64>().ok()) {
            if let Ok(row) = prs::load_pr(conn, repo.id, n) {
                if row.github_number.is_none() {
                    let linked: PrRow = diesel::update(pull_requests::table.find(row.id))
                        .set(pull_requests::github_number.eq(gh.number as i32))
                        .returning(PrRow::as_returning())
                        .get_result(conn)?;
                    existing = Some(linked);
                }
            }
        }
    }
    let dir = vcs::hydrate(app, conn, repo)?;
    let needs_objects = gh.state == "open" || existing.is_none();
    if needs_objects {
        match ensure_pull_objects(app, conn, org, repo, state, &dir, gh.number, &head) {
            Ok(()) => {}
            // A closed pull request is history: its row is worth having
            // even when GitHub no longer serves its head commit.
            Err(e) if gh.state != "open" => {
                tracing::warn!(pr = gh.number, error = %e.message, "head of a closed pull request not fetched; imported without it");
            }
            Err(e) => return Err(e),
        }
        // Only an open pull request brings its branch: a closed one's
        // branch is usually deleted on GitHub, and recreating it here
        // would push it back there.
        if same_repo && gh.state == "open" {
            import_head_branch(app, conn, org, repo, state, &dir, actor, &head_ref, &head)?;
        }
    }
    let base_now = refs::get(conn, repo.id, &base_ref)?
        .map(|r| r.0)
        .unwrap_or_else(|| gh.base_sha.clone());
    let mut events: Vec<&'static str> = Vec::new();
    let row = match existing {
        None => {
            let author = github_link::identity_for_user(conn, &gh.user)?;
            let merged_by = match (&gh.merged_by, gh.state) {
                (Some(u), "merged") => Some(github_link::identity_for_user(conn, u)?.id),
                _ => None,
            };
            let number = number_for_import(conn, repo.id, gh.number)?;
            let created = rfc3339(j, "created_at").unwrap_or_else(Utc::now);
            let closed = (gh.state != "open").then(|| {
                rfc3339(j, "merged_at")
                    .or_else(|| rfc3339(j, "closed_at"))
                    .unwrap_or_else(Utc::now)
            });
            let row: PrRow = diesel::insert_into(pull_requests::table)
                .values((
                    pull_requests::id.eq(vcsstar_db::new_id()),
                    pull_requests::repo_id.eq(repo.id),
                    pull_requests::number.eq(number),
                    pull_requests::title.eq(gh.title.trim()),
                    pull_requests::body.eq((!gh.body.is_empty()).then_some(gh.body.as_str())),
                    pull_requests::author_identity_id.eq(author.id),
                    pull_requests::head_repo_id.eq(same_repo.then_some(repo.id)),
                    pull_requests::head_remote.eq((!same_repo).then(|| {
                        format!(
                            "github:{}",
                            gh.head_repo.clone().unwrap_or_else(|| "<deleted>".into())
                        )
                    })),
                    pull_requests::head_ref.eq(&head_ref),
                    pull_requests::base_ref.eq(&base_ref),
                    pull_requests::head_sha.eq(&gh.head_sha),
                    // A closed one keeps GitHub's base of its time, not
                    // today's base branch.
                    pull_requests::base_sha.eq(if gh.state == "open" || gh.base_sha.is_empty() {
                        &base_now
                    } else {
                        &gh.base_sha
                    }),
                    pull_requests::state.eq(gh.state),
                    pull_requests::merged_commit_sha.eq((gh.state == "merged")
                        .then(|| gh.merge_commit_sha.clone())
                        .flatten()),
                    pull_requests::merged_by.eq(merged_by),
                    pull_requests::closed_at.eq(closed),
                    pull_requests::created_at.eq(created),
                    pull_requests::updated_at.eq(incoming.unwrap_or(created)),
                    pull_requests::github_number.eq(gh.number as i32),
                    pull_requests::github_updated_at.eq(incoming),
                ))
                .returning(PrRow::as_returning())
                .get_result(conn)?;
            let row = if gh.state == "open" {
                refs::apply(
                    conn,
                    repo.id,
                    &[RefUpdate {
                        name: prs::head_ref_name(number),
                        old: None,
                        new: Some(head.clone()),
                    }],
                    Some(actor.identity.id),
                )?;
                conn.transaction::<PrRow, Error, _>(|conn| {
                    prs::trial_merge(app, conn, repo, &dir, &row)
                })?
            } else {
                prs::settle_history_base(app, conn, &dir, &row)?
            };
            events.push(match gh.state {
                "open" => "pr_opened",
                "merged" => "pr_merged",
                _ => "pr_closed",
            });
            row
        }
        Some(cur) => {
            if let (Some(seen), Some(inc)) = (cur.github_updated_at, incoming) {
                if inc < seen {
                    tracing::info!(pr = cur.number, incoming = %inc, applied = %seen, "stale pull request snapshot ignored");
                    return Ok((cur, vec![]));
                }
            }
            let mut row = cur.clone();
            // Born on GitHub (no vcsstar marker in its body): title and
            // body follow GitHub. A pull request vcsstar created keeps
            // vcsstar's text.
            let raw = j.get("body").and_then(Json::as_str).unwrap_or("");
            let github_owned = find_marker(raw, "pr").is_none();
            if github_owned {
                let body_now = cur.body.clone().unwrap_or_default();
                if cur.title.trim() != gh.title.trim() || normalize(&body_now) != gh.body {
                    row = diesel::update(pull_requests::table.find(cur.id))
                        .set((
                            pull_requests::title.eq(gh.title.trim()),
                            pull_requests::body
                                .eq((!gh.body.is_empty()).then_some(gh.body.as_str())),
                            pull_requests::updated_at.eq(Utc::now()),
                        ))
                        .returning(PrRow::as_returning())
                        .get_result(conn)?;
                    events.push("pr_updated");
                }
            }
            let mut retrial = false;
            if row.state == "open" && gh.state == "open" && row.base_ref != base_ref {
                row = diesel::update(pull_requests::table.find(cur.id))
                    .set((
                        pull_requests::base_ref.eq(&base_ref),
                        pull_requests::base_sha.eq(&base_now),
                        pull_requests::mergeable.eq("unknown"),
                        pull_requests::updated_at.eq(Utc::now()),
                    ))
                    .returning(PrRow::as_returning())
                    .get_result(conn)?;
                events.push("pr_retargeted");
                retrial = true;
            }
            if row.state == "open" && gh.state == "open" && row.head_sha != gh.head_sha {
                let current = refs::get(conn, repo.id, &prs::head_ref_name(row.number))?;
                refs::apply(
                    conn,
                    repo.id,
                    &[RefUpdate {
                        name: prs::head_ref_name(row.number),
                        old: current,
                        new: Some(head.clone()),
                    }],
                    Some(actor.identity.id),
                )?;
                row = diesel::update(pull_requests::table.find(cur.id))
                    .set((
                        pull_requests::head_sha.eq(&gh.head_sha),
                        pull_requests::mergeable.eq("unknown"),
                        pull_requests::updated_at.eq(Utc::now()),
                    ))
                    .returning(PrRow::as_returning())
                    .get_result(conn)?;
                events.push("pr_head_moved");
                retrial = true;
            }
            match (row.state.as_str(), gh.state) {
                ("open", "closed") => {
                    row = diesel::update(pull_requests::table.find(cur.id))
                        .set((
                            pull_requests::state.eq("closed"),
                            pull_requests::closed_at.eq(Some(Utc::now())),
                            pull_requests::updated_at.eq(Utc::now()),
                        ))
                        .returning(PrRow::as_returning())
                        .get_result(conn)?;
                    prs::drop_internal_refs(conn, repo.id, row.number)?;
                    events.push("pr_closed");
                    retrial = false;
                }
                ("open", "merged") | ("closed", "merged") => {
                    let merged_by = match &gh.merged_by {
                        Some(u) => Some(github_link::identity_for_user(conn, u)?.id),
                        None => Some(actor.identity.id),
                    };
                    row = diesel::update(pull_requests::table.find(cur.id))
                        .set((
                            pull_requests::state.eq("merged"),
                            pull_requests::merged_commit_sha.eq(gh.merge_commit_sha.clone()),
                            pull_requests::merged_by.eq(merged_by),
                            pull_requests::mergeable.eq("clean"),
                            pull_requests::closed_at.eq(Some(Utc::now())),
                            pull_requests::updated_at.eq(Utc::now()),
                        ))
                        .returning(PrRow::as_returning())
                        .get_result(conn)?;
                    prs::drop_internal_refs(conn, repo.id, row.number)?;
                    events.push("pr_merged");
                    retrial = false;
                }
                ("closed", "open") => {
                    let current = refs::get(conn, repo.id, &prs::head_ref_name(row.number))?;
                    refs::apply(
                        conn,
                        repo.id,
                        &[RefUpdate {
                            name: prs::head_ref_name(row.number),
                            old: current,
                            new: Some(head.clone()),
                        }],
                        Some(actor.identity.id),
                    )?;
                    row = diesel::update(pull_requests::table.find(cur.id))
                        .set((
                            pull_requests::state.eq("open"),
                            pull_requests::closed_at.eq(None::<chrono::DateTime<Utc>>),
                            pull_requests::head_sha.eq(&gh.head_sha),
                            pull_requests::base_sha.eq(&base_now),
                            pull_requests::mergeable.eq("unknown"),
                            pull_requests::updated_at.eq(Utc::now()),
                        ))
                        .returning(PrRow::as_returning())
                        .get_result(conn)?;
                    events.push("pr_reopened");
                    retrial = true;
                }
                _ => {}
            }
            if retrial && row.state == "open" {
                row = conn.transaction::<PrRow, Error, _>(|conn| {
                    prs::trial_merge(app, conn, repo, &dir, &row)
                })?;
            }
            if incoming.is_some() && incoming > row.github_updated_at {
                row = diesel::update(pull_requests::table.find(row.id))
                    .set(pull_requests::github_updated_at.eq(incoming))
                    .returning(PrRow::as_returning())
                    .get_result(conn)?;
            }
            row
        }
    };
    for kind in &events {
        outbox::emit(conn, prs::pr_event(org, repo, Some(actor), kind, &row))?;
    }
    Ok((row, events))
}

fn ingest_pull_request(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    body: &Json,
) -> Result<String, Error> {
    let actor = shadow_principal(sender(conn, body)?);
    let pr = body
        .get("pull_request")
        .ok_or_else(|| Error::invalid("payload without pull_request"))?;
    let action = body.get("action").and_then(Json::as_str).unwrap_or("");
    let (row, events) = upsert_pull(app, conn, org, repo, state, pr, &actor)?;
    Ok(format!(
        "pull_request {action}: #{} (GitHub #{}) {}",
        row.number,
        row.github_number.unwrap_or(0),
        if events.is_empty() {
            "unchanged".to_string()
        } else {
            events.join(",")
        }
    ))
}

/// The vcsstar PR for a GitHub number, importing it through the REST API
/// when no `pull_request` payload came first (hooks added late).
#[allow(clippy::too_many_arguments)]
fn pr_for_github_number(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    number: i64,
    actor: &Principal,
    inline: Option<&Json>,
) -> Result<PrRow, Error> {
    if let Some(j) = inline {
        return Ok(upsert_pull(app, conn, org, repo, state, j, actor)?.0);
    }
    if let Some(row) = pr_by_github_number(conn, repo.id, number)? {
        return Ok(row);
    }
    let (label, token) =
        mirror::cred::token_for(app, conn, org.id, repo.id, "mirror_api").map_err(Error::from)?;
    let gh = github::Client::new(&app.config.github_api_base, &token).with_label(&label);
    let j = gh
        .get_pull(&state.github_repo, number)?
        .ok_or_else(|| Error::not_found(&format!("GitHub pull request #{number}")))?;
    Ok(upsert_pull(app, conn, org, repo, state, &j, actor)?.0)
}

fn ingest_review(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    body: &Json,
) -> Result<String, Error> {
    let action = body.get("action").and_then(Json::as_str).unwrap_or("");
    if action != "submitted" {
        return Ok(format!("ignored review action {action}"));
    }
    let actor = shadow_principal(sender(conn, body)?);
    let pr_json = body.get("pull_request");
    let number = pr_json
        .and_then(|p| p.get("number"))
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("review payload without pull_request.number"))?;
    let pr = pr_for_github_number(app, conn, org, repo, state, number, &actor, pr_json)?;
    let review = body
        .get("review")
        .ok_or_else(|| Error::invalid("payload without review"))?;
    upsert_review(conn, org, repo, &pr, review)
}

/// Move a pull request's `updated_at` forward to `at`, never back: an
/// imported old review or comment does not make the pull request look
/// recent.
fn touch_pull(
    conn: &mut PgConnection,
    pr_id: Uuid,
    at: chrono::DateTime<Utc>,
) -> Result<(), Error> {
    diesel::update(
        pull_requests::table
            .find(pr_id)
            .filter(pull_requests::updated_at.lt(at)),
    )
    .set(pull_requests::updated_at.eq(at))
    .execute(conn)?;
    Ok(())
}

/// Import one GitHub review object (webhook `review` or a REST list item)
/// when it is not known yet. Returns a note; "imported" when a row was made.
fn upsert_review(
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    pr: &PrRow,
    review: &Json,
) -> Result<String, Error> {
    let gh_id = review
        .get("id")
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("review without id"))?;
    let exists: Option<Uuid> = pr_reviews::table
        .filter(pr_reviews::pr_id.eq(pr.id))
        .filter(pr_reviews::github_id.eq(gh_id))
        .select(pr_reviews::id)
        .first(conn)
        .optional()?;
    if exists.is_some() {
        return Ok(format!("review {gh_id} already imported"));
    }
    let raw = review.get("body").and_then(Json::as_str).unwrap_or("");
    if let Some(id) = find_marker(raw, "review").and_then(|s| Uuid::parse_str(&s).ok()) {
        let n = diesel::update(
            pr_reviews::table
                .find(id)
                .filter(pr_reviews::pr_id.eq(pr.id)),
        )
        .set(pr_reviews::github_id.eq(gh_id))
        .execute(conn)?;
        if n == 1 {
            return Ok(format!("review {gh_id} linked to vcsstar review {id}"));
        }
    }
    let st = match review
        .get("state")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "approved" => "approved",
        "changes_requested" => "changes_requested",
        "commented" => "comment",
        other => return Ok(format!("ignored review state {other}")),
    };
    let reviewer = github_link::identity_for_user(conn, review.get("user").unwrap_or(&Json::Null))?;
    let head_sha = review
        .get("commit_id")
        .and_then(Json::as_str)
        .unwrap_or(&pr.head_sha)
        .to_string();
    let text = strip_vcsstar(raw);
    let submitted = rfc3339(review, "submitted_at").unwrap_or_else(Utc::now);
    if st == "comment" && text.is_empty() {
        // The review GitHub makes around an inline comment carries nothing
        // of its own; the comment arrives as pull_request_review_comment.
        return Ok(format!(
            "review {gh_id} is an empty comment review; ignored"
        ));
    }
    let row: ReviewRow = conn.transaction::<ReviewRow, Error, _>(|conn| {
        let row: ReviewRow = diesel::insert_into(pr_reviews::table)
            .values((
                pr_reviews::id.eq(vcsstar_db::new_id()),
                pr_reviews::pr_id.eq(pr.id),
                pr_reviews::reviewer.eq(reviewer.id),
                pr_reviews::state.eq(st),
                pr_reviews::head_sha_at_review.eq(&head_sha),
                pr_reviews::body.eq((!text.is_empty()).then_some(text.as_str())),
                pr_reviews::github_id.eq(gh_id),
                pr_reviews::created_at.eq(submitted),
            ))
            .returning(ReviewRow::as_returning())
            .get_result(conn)?;
        touch_pull(conn, pr.id, submitted)?;
        let reviewer_p = shadow_principal(reviewer.clone());
        outbox::emit(
            conn,
            prs::pr_event_about(
                org,
                repo,
                Some(&reviewer_p),
                "pr_review",
                pr,
                None,
                Some(row.id),
            ),
        )?;
        Ok(row)
    })?;
    Ok(format!("review {gh_id} imported as {} ({st})", row.id))
}

/// Shared by conversation and inline comments. `kind` is `issue` or `review`.
#[allow(clippy::too_many_arguments)]
fn upsert_comment(
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    pr: &PrRow,
    action: &str,
    comment: &Json,
    kind: &str,
    location: Option<(String, Option<i64>, Option<String>)>,
) -> Result<String, Error> {
    let gh_id = comment
        .get("id")
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("comment without id"))?;
    let raw = comment.get("body").and_then(Json::as_str).unwrap_or("");
    let existing: Option<CommentRow> = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .filter(pr_comments::github_id.eq(gh_id))
        .select(CommentRow::as_select())
        .first(conn)
        .optional()?;
    let author = github_link::identity_for_user(conn, comment.get("user").unwrap_or(&Json::Null))?;
    let actor = shadow_principal(author.clone());
    let incoming = rfc3339(comment, "updated_at");
    match (action, existing) {
        ("deleted", Some(row)) => {
            diesel::delete(pr_comments::table.find(row.id)).execute(conn)?;
            outbox::emit(
                conn,
                prs::pr_event_about(
                    org,
                    repo,
                    Some(&actor),
                    "pr_comment_deleted",
                    pr,
                    Some(row.id),
                    None,
                ),
            )?;
            Ok(format!("comment {gh_id} deleted"))
        }
        ("deleted", None) => Ok(format!("comment {gh_id} unknown; nothing to delete")),
        (_, Some(row)) => {
            // The last writer by GitHub's clock wins, wherever the comment
            // was born: vcsstar has no edit of its own that could re-assert
            // its text, and a linked user who edits their comment on GitHub
            // expects it to follow. A snapshot older than the one applied
            // (redelivery, out of order) is ignored. The event carries
            // origin `github`, so nothing goes back.
            if let (Some(seen), Some(inc)) = (row.github_updated_at, incoming) {
                if inc < seen {
                    return Ok(format!(
                        "comment {gh_id} snapshot ({inc}) is older than the applied one ({seen}); ignored"
                    ));
                }
            }
            let text = strip_vcsstar(raw);
            if normalize(&row.body) == text {
                if incoming.is_some() && incoming > row.github_updated_at {
                    diesel::update(pr_comments::table.find(row.id))
                        .set(pr_comments::github_updated_at.eq(incoming))
                        .execute(conn)?;
                }
                return Ok(format!("comment {gh_id} unchanged"));
            }
            diesel::update(pr_comments::table.find(row.id))
                .set((
                    pr_comments::body.eq(&text),
                    pr_comments::edited_at.eq(Some(Utc::now())),
                    pr_comments::github_updated_at.eq(incoming.or(row.github_updated_at)),
                ))
                .execute(conn)?;
            outbox::emit(
                conn,
                prs::pr_event_about(
                    org,
                    repo,
                    Some(&actor),
                    "pr_comment_edited",
                    pr,
                    Some(row.id),
                    None,
                ),
            )?;
            Ok(format!("comment {gh_id} edited"))
        }
        (_, None) => {
            if find_marker(raw, "pr").is_some() || find_marker(raw, "review").is_some() {
                // A note vcsstar itself posted (merge notice, review fallback).
                return Ok(format!(
                    "comment {gh_id} is vcsstar's own note; not imported"
                ));
            }
            if let Some(id) = find_marker(raw, "comment").and_then(|s| Uuid::parse_str(&s).ok()) {
                let n = diesel::update(
                    pr_comments::table
                        .find(id)
                        .filter(pr_comments::pr_id.eq(pr.id)),
                )
                .set((
                    pr_comments::github_id.eq(gh_id),
                    pr_comments::github_kind.eq(kind),
                ))
                .execute(conn)?;
                if n == 1 {
                    return Ok(format!("comment {gh_id} linked to vcsstar comment {id}"));
                }
            }
            let text = strip_vcsstar(raw);
            if text.is_empty() {
                return Ok(format!("comment {gh_id} is empty; ignored"));
            }
            let (path, line, commit_sha) = location.unwrap_or((String::new(), None, None));
            let created = rfc3339(comment, "created_at").unwrap_or_else(Utc::now);
            let row: CommentRow = conn.transaction::<CommentRow, Error, _>(|conn| {
                let row: CommentRow = diesel::insert_into(pr_comments::table)
                    .values((
                        pr_comments::id.eq(vcsstar_db::new_id()),
                        pr_comments::pr_id.eq(pr.id),
                        pr_comments::author.eq(author.id),
                        pr_comments::body.eq(&text),
                        pr_comments::path.eq((!path.is_empty()).then_some(path.as_str())),
                        pr_comments::line.eq(line.map(|l| l.clamp(0, i32::MAX as i64) as i32)),
                        pr_comments::commit_sha.eq(commit_sha.as_deref()),
                        pr_comments::github_id.eq(gh_id),
                        pr_comments::github_kind.eq(kind),
                        pr_comments::github_updated_at.eq(incoming),
                        pr_comments::created_at.eq(created),
                    ))
                    .returning(CommentRow::as_returning())
                    .get_result(conn)?;
                touch_pull(conn, pr.id, created)?;
                outbox::emit(
                    conn,
                    prs::pr_event_about(
                        org,
                        repo,
                        Some(&actor),
                        "pr_comment",
                        pr,
                        Some(row.id),
                        None,
                    ),
                )?;
                Ok(row)
            })?;
            Ok(format!("comment {gh_id} imported as {}", row.id))
        }
    }
}

fn ingest_issue_comment(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    body: &Json,
) -> Result<String, Error> {
    let issue = body
        .get("issue")
        .ok_or_else(|| Error::invalid("payload without issue"))?;
    if issue.get("pull_request").is_none() {
        return Ok("ignored: comment on an issue, not a pull request".into());
    }
    let number = issue
        .get("number")
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("issue without number"))?;
    let actor = shadow_principal(sender(conn, body)?);
    let pr = pr_for_github_number(app, conn, org, repo, state, number, &actor, None)?;
    let action = body
        .get("action")
        .and_then(Json::as_str)
        .unwrap_or("created");
    let comment = body
        .get("comment")
        .ok_or_else(|| Error::invalid("payload without comment"))?;
    upsert_comment(conn, org, repo, &pr, action, comment, "issue", None)
}

fn ingest_review_comment(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    body: &Json,
) -> Result<String, Error> {
    let actor = shadow_principal(sender(conn, body)?);
    let pr_json = body.get("pull_request");
    let number = pr_json
        .and_then(|p| p.get("number"))
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("payload without pull_request.number"))?;
    let pr = pr_for_github_number(app, conn, org, repo, state, number, &actor, pr_json)?;
    let action = body
        .get("action")
        .and_then(Json::as_str)
        .unwrap_or("created");
    let comment = body
        .get("comment")
        .ok_or_else(|| Error::invalid("payload without comment"))?;
    let path = comment
        .get("path")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let line = comment
        .get("line")
        .and_then(Json::as_i64)
        .or_else(|| comment.get("original_line").and_then(Json::as_i64));
    let sha = comment
        .get("commit_id")
        .and_then(Json::as_str)
        .map(str::to_string);
    upsert_comment(
        conn,
        org,
        repo,
        &pr,
        action,
        comment,
        "review",
        Some((path, line, sha)),
    )
}

/// Upsert one commit status reported by GitHub. Returns whether the row
/// changed; an unchanged row emits nothing (that is the echo of our own
/// status coming back).
#[allow(clippy::too_many_arguments)]
fn upsert_status(
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    reporter: &Identity,
    sha: &str,
    context: &str,
    state: &str,
    target_url: Option<&str>,
    description: Option<&str>,
) -> Result<bool, Error> {
    let existing: Option<StatusRow> = statuses::table
        .filter(statuses::repo_id.eq(repo.id))
        .filter(statuses::sha.eq(sha))
        .filter(statuses::context.eq(context))
        .select(StatusRow::as_select())
        .first(conn)
        .optional()?;
    if let Some(e) = &existing {
        if e.state == state
            && e.target_url.as_deref() == target_url
            && e.description.as_deref() == description
        {
            return Ok(false);
        }
    }
    let actor = shadow_principal(reporter.clone());
    conn.transaction::<(), Error, _>(|conn| {
        diesel::insert_into(statuses::table)
            .values((
                statuses::id.eq(vcsstar_db::new_id()),
                statuses::repo_id.eq(repo.id),
                statuses::sha.eq(sha),
                statuses::context.eq(context),
                statuses::state.eq(state),
                statuses::target_url.eq(target_url),
                statuses::description.eq(description),
                statuses::reporter.eq(reporter.id),
            ))
            .on_conflict((statuses::repo_id, statuses::sha, statuses::context))
            .do_update()
            .set((
                statuses::state.eq(state),
                statuses::target_url.eq(target_url),
                statuses::description.eq(description),
                statuses::reporter.eq(reporter.id),
                statuses::created_at.eq(Utc::now()),
            ))
            .execute(conn)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_repo(&org.name, &repo.name),
                kind: "status_set",
                repo_id: Some(repo.id),
                org_id: Some(org.id),
                actor_id: Some(actor.identity.id),
                actor: Some(actor.to_wire()),
                repo: Some(vcs::repo_ref(org, repo)),
                org: Some(org.name.clone()),
                subject: Some(sha.to_string()),
                payload: EventPayload::Variant6(StatusPayload {
                    sha: sha.to_string(),
                    context: context.to_string(),
                    state: state.to_string(),
                }),
            },
        )?;
        Ok(())
    })?;
    Ok(true)
}

fn ingest_status(conn: &mut DbConn, org: &Org, repo: &Repo, body: &Json) -> Result<String, Error> {
    let s = |k: &str| body.get(k).and_then(Json::as_str);
    let sha = s("sha").ok_or_else(|| Error::invalid("status without sha"))?;
    let context = s("context").ok_or_else(|| Error::invalid("status without context"))?;
    let state = match s("state") {
        Some(v @ ("pending" | "success" | "failure" | "error")) => v,
        other => return Ok(format!("ignored status state {other:?}")),
    };
    let reporter = sender(conn, body)?;
    let changed = upsert_status(
        conn,
        org,
        repo,
        &reporter,
        sha,
        context,
        state,
        s("target_url"),
        s("description"),
    )?;
    Ok(format!(
        "status {context} on {} {}",
        short(sha),
        if changed { state } else { "unchanged" }
    ))
}

/// Map a check run (GitHub Actions and other Checks apps) to a commit status.
pub fn check_run_state(status: &str, conclusion: Option<&str>) -> &'static str {
    if status != "completed" {
        return "pending";
    }
    match conclusion.unwrap_or("") {
        "success" | "neutral" | "skipped" => "success",
        "failure" | "timed_out" | "cancelled" | "startup_failure" => "failure",
        _ => "error",
    }
}

fn ingest_check_run(
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    body: &Json,
) -> Result<String, Error> {
    let run = body
        .get("check_run")
        .ok_or_else(|| Error::invalid("payload without check_run"))?;
    let s = |p: &str| run.pointer(p).and_then(Json::as_str);
    let sha = s("/head_sha").ok_or_else(|| Error::invalid("check_run without head_sha"))?;
    let name = s("/name").ok_or_else(|| Error::invalid("check_run without name"))?;
    let state = check_run_state(s("/status").unwrap_or(""), s("/conclusion"));
    let description = s("/output/title").or_else(|| s("/output/summary"));
    let reporter = sender(conn, body)?;
    let changed = upsert_status(
        conn,
        org,
        repo,
        &reporter,
        sha,
        name,
        state,
        s("/html_url"),
        description,
    )?;
    Ok(format!(
        "check_run {name} on {} {}",
        short(sha),
        if changed { state } else { "unchanged" }
    ))
}

// ---------------------------------------------------------------------
// Pull-based sync: GitHub's state -> vcsstar rows, without webhooks
// ---------------------------------------------------------------------

/// What one pull-based pass did.
#[derive(Debug, Default, Clone)]
pub struct PullSyncOutcome {
    /// Pull requests looked at.
    pub checked: usize,
    /// Pull requests, reviews, comments, or statuses that changed here.
    pub changed: usize,
    /// GitHub API calls spent.
    pub calls: usize,
    /// False when the call budget or a rate limit hold stopped the pass;
    /// the watermark did not move and the next pass continues.
    pub complete: bool,
}

/// Calls one pass may spend on a repository.
pub const PULL_SYNC_BUDGET: usize = 150;

/// Reason a pass stopped early.
enum Budget {
    Spent,
    RateHeld(std::time::Duration),
}

struct Meter<'a> {
    gh: &'a github::Client,
    calls: usize,
    budget: usize,
    /// Bulk work (the history import) keeps a larger rate reserve.
    bulk: bool,
}

impl Meter<'_> {
    /// Account for one call, or say why it may not be made.
    fn spend(&mut self) -> Result<(), Budget> {
        if self.calls >= self.budget {
            return Err(Budget::Spent);
        }
        let hold = if self.bulk {
            crate::github_health::bulk_hold(self.gh.label())
        } else {
            self.gh.rate_hold()
        };
        if let Some(wait) = hold {
            return Err(Budget::RateHeld(wait));
        }
        self.calls += 1;
        Ok(())
    }
}

fn rfc3339(j: &Json, key: &str) -> Option<chrono::DateTime<Utc>> {
    j.get(key)
        .and_then(Json::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// Bring vcsstar up to date with GitHub's pull requests by asking GitHub,
/// not by waiting for webhooks. Webhooks are lost during an outage (GitHub
/// does not retry them) and may be missed any time; this pass is the
/// floor under them. It lists the repository's pull requests by
/// `updated_at`, newest first, stops at the last watermark, and replays
/// every newer one through the same upserts the webhook path uses, plus
/// its reviews, comments, statuses, and check runs. Rows that already
/// match change nothing and emit nothing. Everything it writes carries
/// origin `github`, so nothing is sent back.
///
/// The first pass of a repository takes its *open* pull requests only and
/// sets the watermark to the start of the pass; history is not imported.
/// A pass stops at `budget` API calls or at a rate limit hold and leaves
/// the watermark where it was (the next pass redoes the newest, which is
/// idempotent). Deletions made on GitHub are not detected by this pass.
pub fn reconcile_pulls(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    budget: usize,
) -> Result<PullSyncOutcome, Error> {
    outbox::with_origin("github", || {
        reconcile_pulls_inner(app, conn, org, repo, state, budget)
    })
}

fn reconcile_pulls_inner(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    budget: usize,
) -> Result<PullSyncOutcome, Error> {
    let token = mirror::cred::token_for(app, conn, org.id, repo.id, "mirror_api")?;
    let gh = github::Client::new(&app.config.github_api_base, &token.1).with_label(&token.0);
    let full = state.github_repo.as_str();
    let since = state.last_pull_sync_at;
    let started = Utc::now();
    let mut out = PullSyncOutcome::default();
    let mut meter = Meter {
        gh: &gh,
        calls: 0,
        budget,
        bulk: false,
    };
    let list_state = if since.is_some() { "all" } else { "open" };
    // Collect the pull requests newer than the watermark, newest first.
    let mut pulls: Vec<Json> = Vec::new();
    let mut page = 1u32;
    let mut stopped: Option<Budget> = None;
    'pages: loop {
        if let Err(b) = meter.spend() {
            stopped = Some(b);
            break;
        }
        let (items, more) = gh.list_pulls(full, list_state, page, 100)?;
        for item in items {
            if let (Some(w), Some(u)) = (since, rfc3339(&item, "updated_at")) {
                if u < w {
                    break 'pages;
                }
            }
            pulls.push(item);
        }
        if !more {
            break;
        }
        page += 1;
    }
    // Oldest first, so the events land in the order GitHub saw them.
    pulls.reverse();
    let since_text = since.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    // Newest `updated_at` fully replayed in this pass. The list is oldest
    // first, so a pass that stops early may still move the watermark up
    // to here, and a long backlog drains over several passes.
    let mut replayed: Option<chrono::DateTime<Utc>> = None;
    if stopped.is_none() {
        for j in &pulls {
            match sync_one_pull(
                app,
                conn,
                org,
                repo,
                state,
                &gh,
                &mut meter,
                j,
                since_text.as_deref(),
            ) {
                Ok(changed) => {
                    out.checked += 1;
                    out.changed += changed;
                    replayed = std::cmp::max(replayed, rfc3339(j, "updated_at"));
                }
                Err(SyncStop::Budget(b)) => {
                    stopped = Some(b);
                    break;
                }
                Err(SyncStop::Error(e)) => return Err(e),
            }
        }
    }
    if stopped.is_some() {
        // Keep what was done (a later pass redoes only the rest). Not on a
        // first pass: it lists open pull requests only, so a partial one
        // proves nothing about the others.
        if let (Some(w), Some(r)) = (since, replayed) {
            if r > w {
                diesel::update(mirror::mirror_state_table().find(repo.id))
                    .set(vcsstar_db::schema::mirror_state::last_pull_sync_at.eq(r))
                    .execute(conn)?;
            }
        }
    }
    out.calls = meter.calls;
    out.complete = stopped.is_none();
    match stopped {
        None => {
            // Newest `updated_at` seen, or the start of this pass when the
            // repository had nothing newer (and on the first pass).
            let newest = pulls.iter().filter_map(|j| rfc3339(j, "updated_at")).max();
            let watermark = match (since, newest) {
                (None, _) => started,
                (Some(w), Some(n)) => std::cmp::max(w, n),
                (Some(w), None) => w,
            };
            diesel::update(mirror::mirror_state_table().find(repo.id))
                .set(vcsstar_db::schema::mirror_state::last_pull_sync_at.eq(watermark))
                .execute(conn)?;
            crate::metrics::github_pull_sync("complete");
        }
        Some(Budget::Spent) => {
            tracing::info!(repo = %full, calls = out.calls, "pull sync stopped at its call budget; the next pass continues");
            crate::metrics::github_pull_sync("partial");
        }
        Some(Budget::RateHeld(wait)) => {
            tracing::info!(repo = %full, wait_s = wait.as_secs(), token = %gh.label(), "pull sync stopped: rate limit reserve reached");
            crate::metrics::github_pull_sync("held");
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// History import: every pull request GitHub has, oldest first
// ---------------------------------------------------------------------

/// What one history import pass did.
#[derive(Debug, Default, Clone)]
pub struct BackfillOutcome {
    /// Pull requests gone through in this pass.
    pub pulls: usize,
    /// Rows that changed here.
    pub changed: usize,
    /// GitHub API calls spent.
    pub calls: usize,
    /// The import reached the end of GitHub's list.
    pub done: bool,
    /// A rate limit stopped the pass: start again after this long.
    pub held: Option<std::time::Duration>,
    /// Pull requests skipped because they could not be imported.
    pub skipped: usize,
}

/// Calls one history import pass may spend before it lets the next
/// repository have a turn. The rate reserve, not this budget, decides how
/// fast an import goes.
pub const BACKFILL_BUDGET: usize = 300;

/// A position that fails this many passes in a row is skipped.
const BACKFILL_STUCK_AFTER: u32 = 3;

/// Import a repository's GitHub pull requests from the first one ever
/// opened, with their reviews, comments, and (for open ones) statuses and
/// check runs, through the same upserts as webhooks. The position is kept
/// in `mirror_state` after every pull request, so a pass that stops (call
/// budget, rate limit reserve, restart) loses nothing; the next pass goes
/// on from there. Rows that already match change nothing. Everything it
/// writes carries origin `github`, so nothing is sent back. Live work has
/// priority: the import keeps a larger rate reserve (`bulk_hold`).
///
/// Before the first page every `refs/pull/*/head` is fetched in one go and
/// recorded as one segment, so old pull requests need no fetch each.
pub fn backfill_pulls(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    budget: usize,
) -> Result<BackfillOutcome, Error> {
    outbox::with_origin("github", || {
        backfill_inner(app, conn, org, repo, state, budget)
    })
}

fn backfill_inner(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    budget: usize,
) -> Result<BackfillOutcome, Error> {
    use vcsstar_db::schema::mirror_state as ms;
    let (label, token) = mirror::cred::token_for(app, conn, org.id, repo.id, "mirror_api")?;
    let gh = github::Client::new(&app.config.github_api_base, &token).with_label(&label);
    let full = state.github_repo.as_str();
    let mut out = BackfillOutcome::default();
    if state.backfill_started_at.is_none() {
        diesel::update(mirror::mirror_state_table().find(repo.id))
            .set(ms::backfill_started_at.eq(Some(Utc::now())))
            .execute(conn)?;
    }
    if state.backfill_heads_at.is_none() {
        let dir = vcs::hydrate(app, conn, repo)?;
        let fetch_token = mirror_token(app, conn, org, repo);
        let url = mirror::github_url(app, full);
        let heads = mirror::fetch_pull_heads(&dir, &url, fetch_token.as_deref())?;
        if !heads.is_empty() {
            let have: Vec<RevId> = refs::load(conn, repo.id)?
                .into_iter()
                .map(|(_, t)| t)
                .collect();
            vcs::capture(app, conn, repo.id, &dir, &have, &heads)?;
        }
        tracing::info!(repo = %full, heads = heads.len(), "history import: pull request heads fetched");
        diesel::update(mirror::mirror_state_table().find(repo.id))
            .set(ms::backfill_heads_at.eq(Some(Utc::now())))
            .execute(conn)?;
    }
    let mut meter = Meter {
        gh: &gh,
        calls: 0,
        budget,
        bulk: true,
    };
    let mut page = state.backfill_page.max(1) as u32;
    let mut index = state.backfill_index.max(0) as usize;
    let mut total = state.backfill_pulls.max(0);
    let save = |conn: &mut DbConn, page: u32, index: usize, total: i32| {
        diesel::update(mirror::mirror_state_table().find(repo.id))
            .set((
                ms::backfill_page.eq(page as i32),
                ms::backfill_index.eq(index as i32),
                ms::backfill_pulls.eq(total),
            ))
            .execute(conn)
    };
    let mut stopped: Option<Budget> = None;
    'pages: loop {
        if let Err(b) = meter.spend() {
            stopped = Some(b);
            break;
        }
        let (items, more) = gh.list_pulls_created(full, page, 100)?;
        while index < items.len() {
            match sync_one_pull(
                app,
                conn,
                org,
                repo,
                state,
                &gh,
                &mut meter,
                &items[index],
                None,
            ) {
                Ok(changed) => {
                    out.pulls += 1;
                    out.changed += changed;
                }
                Err(SyncStop::Budget(b)) => {
                    stopped = Some(b);
                    break 'pages;
                }
                Err(SyncStop::Error(e)) => {
                    let number = items[index]
                        .get("number")
                        .and_then(Json::as_i64)
                        .unwrap_or(0);
                    // GitHub or the store is unavailable: try this one
                    // again in the next pass. A pull request that keeps
                    // failing (or fails for good) is skipped and named in
                    // `last_error`, so one bad item never wedges the import.
                    let marker = format!("history import stuck at #{number}");
                    let attempts = state
                        .last_error
                        .as_deref()
                        .filter(|m| m.starts_with(&marker))
                        .and_then(|m| m.rsplit("attempt ").next())
                        .and_then(|n| n.trim_end_matches(')').parse::<u32>().ok())
                        .unwrap_or(0)
                        + 1;
                    if e.code == Code::Unavailable && attempts < BACKFILL_STUCK_AFTER {
                        diesel::update(mirror::mirror_state_table().find(repo.id))
                            .set(
                                ms::last_error
                                    .eq(format!("{marker}: {} (attempt {attempts})", e.message)),
                            )
                            .execute(conn)?;
                        save(conn, page, index, total)?;
                        return Err(e);
                    }
                    tracing::warn!(repo = %full, pr = number, error = %e.message, "history import skipped a pull request");
                    diesel::update(mirror::mirror_state_table().find(repo.id))
                        .set(
                            ms::last_error
                                .eq(format!("history import skipped #{number}: {}", e.message)),
                        )
                        .execute(conn)?;
                    out.skipped += 1;
                }
            }
            index += 1;
            total += 1;
            save(conn, page, index, total)?;
        }
        if !more {
            out.done = true;
            diesel::update(mirror::mirror_state_table().find(repo.id))
                .set(ms::backfill_done_at.eq(Some(Utc::now())))
                .execute(conn)?;
            break;
        }
        page += 1;
        index = 0;
        save(conn, page, index, total)?;
    }
    out.calls = meter.calls;
    match stopped {
        None => {
            tracing::info!(repo = %full, pulls = total, "history import complete");
            crate::metrics::github_backfill("done");
        }
        Some(Budget::Spent) => crate::metrics::github_backfill("partial"),
        Some(Budget::RateHeld(wait)) => {
            tracing::info!(repo = %full, wait_s = wait.as_secs(), token = %gh.label(), "history import paused: rate limit reserve reached");
            out.held = Some(wait);
            crate::metrics::github_backfill("held");
        }
    }
    crate::metrics::github_backfill_pulls(out.pulls as u64);
    Ok(out)
}

/// Start the history import of a repository again from the first pull
/// request (`Mirror/backfill`). Rows already imported are kept and match.
pub fn restart_backfill(conn: &mut PgConnection, repo_id: Uuid) -> Result<(), Error> {
    use vcsstar_db::schema::mirror_state as ms;
    diesel::update(mirror::mirror_state_table().find(repo_id))
        .set((
            ms::backfill_page.eq(1),
            ms::backfill_index.eq(0),
            ms::backfill_pulls.eq(0),
            ms::backfill_heads_at.eq(None::<chrono::DateTime<Utc>>),
            ms::backfill_pass_at.eq(None::<chrono::DateTime<Utc>>),
            ms::backfill_started_at.eq(None::<chrono::DateTime<Utc>>),
            ms::backfill_done_at.eq(None::<chrono::DateTime<Utc>>),
        ))
        .execute(conn)?;
    Ok(())
}

enum SyncStop {
    Budget(Budget),
    Error(Error),
}

impl From<Error> for SyncStop {
    fn from(e: Error) -> Self {
        SyncStop::Error(e)
    }
}
impl From<Failure> for SyncStop {
    fn from(f: Failure) -> Self {
        SyncStop::Error(f.into())
    }
}
impl From<Budget> for SyncStop {
    fn from(b: Budget) -> Self {
        SyncStop::Budget(b)
    }
}
impl From<diesel::result::Error> for SyncStop {
    fn from(e: diesel::result::Error) -> Self {
        SyncStop::Error(e.into())
    }
}

/// Replay one pull request and what hangs off it. Returns how many rows
/// changed.
#[allow(clippy::too_many_arguments)]
fn sync_one_pull(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    gh: &github::Client,
    meter: &mut Meter<'_>,
    j: &Json,
    since: Option<&str>,
) -> Result<usize, SyncStop> {
    let full = state.github_repo.as_str();
    let number = j
        .get("number")
        .and_then(Json::as_i64)
        .ok_or_else(|| Error::invalid("pull request without number"))?;
    let author = github_link::identity_for_user(conn, j.get("user").unwrap_or(&Json::Null))?;
    let actor = shadow_principal(author.clone());
    let (pr, events) = upsert_pull(app, conn, org, repo, state, j, &actor)?;
    let mut changed = usize::from(!events.is_empty());
    let open = pr.state == "open";
    let counts = |note: &str| -> usize {
        usize::from(
            (note.contains("imported") && !note.contains("already imported"))
                || note.contains("edited")
                || note.contains("linked")
                || note.contains("deleted"),
        )
    };

    let mut page = 1u32;
    loop {
        meter.spend()?;
        let (items, more) = gh.list_reviews(full, number, page)?;
        for review in items {
            changed += counts(&upsert_review(conn, org, repo, &pr, &review)?);
        }
        if !more {
            break;
        }
        page += 1;
    }
    let mut page = 1u32;
    loop {
        meter.spend()?;
        let (items, more) = gh.list_issue_comments(full, number, since, page)?;
        for c in items {
            changed += counts(&upsert_comment(
                conn, org, repo, &pr, "synced", &c, "issue", None,
            )?);
        }
        if !more {
            break;
        }
        page += 1;
    }
    let mut review_comments: Vec<Json> = Vec::new();
    let mut page = 1u32;
    loop {
        meter.spend()?;
        let (items, more) = gh.list_review_comments(full, number, since, page)?;
        review_comments.extend(items);
        if !more {
            break;
        }
        page += 1;
    }
    for c in review_comments {
        let path = c
            .get("path")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let line = c
            .get("line")
            .and_then(Json::as_i64)
            .or_else(|| c.get("original_line").and_then(Json::as_i64));
        let sha = c
            .get("commit_id")
            .and_then(Json::as_str)
            .map(str::to_string);
        changed += counts(&upsert_comment(
            conn,
            org,
            repo,
            &pr,
            "synced",
            &c,
            "review",
            Some((path, line, sha)),
        )?);
    }
    if open {
        meter.spend()?;
        for st in gh.combined_status(full, &pr.head_sha)? {
            let s = |k: &str| st.get(k).and_then(Json::as_str);
            let (Some(context), Some(st_state)) = (s("context"), s("state")) else {
                continue;
            };
            if !["pending", "success", "failure", "error"].contains(&st_state) {
                continue;
            }
            let reporter = match st.get("creator").filter(|v| v.is_object()) {
                Some(u) => github_link::identity_for_user(conn, u)?,
                None => author.clone(),
            };
            if upsert_status(
                conn,
                org,
                repo,
                &reporter,
                &pr.head_sha,
                context,
                st_state,
                s("target_url"),
                s("description"),
            )? {
                changed += 1;
            }
        }
        meter.spend()?;
        for run in gh.check_runs(full, &pr.head_sha)? {
            let s = |p: &str| run.pointer(p).and_then(Json::as_str);
            let Some(name) = s("/name") else { continue };
            let st_state = check_run_state(s("/status").unwrap_or(""), s("/conclusion"));
            let reporter = match run.pointer("/app/owner").filter(|v| v.is_object()) {
                Some(u) => github_link::identity_for_user(conn, u)?,
                None => author.clone(),
            };
            if upsert_status(
                conn,
                org,
                repo,
                &reporter,
                &pr.head_sha,
                name,
                st_state,
                s("/html_url"),
                s("/output/title").or_else(|| s("/output/summary")),
            )? {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

// ---------------------------------------------------------------------
// Outbound: mirror actions -> GitHub objects
// ---------------------------------------------------------------------

/// Early exit from an executor: either a decided outcome or an error.
enum Stop {
    Exec(Exec),
    Error(Error),
}

impl From<Failure> for Stop {
    fn from(f: Failure) -> Self {
        Stop::Exec(match f {
            Failure::Terminal(m) => Exec::Blocked(m),
            Failure::Transient(m) => Exec::Transient(m),
        })
    }
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Stop::Error(e)
    }
}

impl From<diesel::result::Error> for Stop {
    fn from(e: diesel::result::Error) -> Self {
        Stop::Error(e.into())
    }
}

impl From<vcsstar_store::StoreError> for Stop {
    fn from(e: vcsstar_store::StoreError) -> Self {
        Stop::Error(e.into())
    }
}

fn finish(r: Result<Exec, Stop>) -> Result<Exec, Error> {
    match r {
        Ok(e) | Err(Stop::Exec(e)) => Ok(e),
        Err(Stop::Error(e)) => Err(e),
    }
}

/// A GitHub client for the action, acting as the event's actor when that
/// identity is linked to GitHub (`used` records which token was chosen).
fn client(
    app: &App,
    conn: &mut PgConnection,
    org: &Org,
    repo: &Repo,
    action: &ActionRow,
    used: &mut Option<Token>,
) -> Result<github::Client, Stop> {
    let actor = mirror::event_actor(conn, action)?;
    let token = mirror::cred::token_for_actor(
        app,
        conn,
        org.id,
        repo.id,
        &action.credential_purpose,
        actor,
    )?;
    tracing::info!(action = %action.object_key, source = %token.label(), "GitHub credential chosen");
    let client =
        github::Client::new(&app.config.github_api_base, &token.value).with_label(&token.label());
    *used = Some(token);
    Ok(client)
}

fn acted_as_user(used: &Option<Token>) -> bool {
    used.as_ref().is_some_and(Token::is_user)
}

/// A blocked outcome caused by a linked user's token being refused is
/// not final: mark the link broken and retry, which then falls back to
/// the instance App or the organization credential.
pub fn settle_refusal(conn: &mut PgConnection, used: &Token, exec: Exec) -> Exec {
    if let (
        Exec::Blocked(m),
        TokenSource::User {
            identity_id, login, ..
        },
    ) = (&exec, &used.source)
    {
        // A user token that cannot see the repository gets 404s from
        // GitHub (it hides what the token may not know), so for a linked
        // user a 404 on a write counts as a refusal too.
        if github::is_auth_refusal(m) || m.contains("HTTP 404") {
            github_link::mark_broken(conn, *identity_id, m);
            return Exec::Transient(format!(
                "GitHub refused {login}'s linked token ({m}); the link is marked broken and the mirror credential is used next"
            ));
        }
    }
    exec
}

fn settle(
    conn: &mut PgConnection,
    used: &Option<Token>,
    r: Result<Exec, Error>,
) -> Result<Exec, Error> {
    match (used, r) {
        (Some(t), Ok(exec)) => Ok(settle_refusal(conn, t, exec)),
        (_, r) => r,
    }
}

fn load_pr_opt(
    conn: &mut PgConnection,
    repo_id: Uuid,
    number: i32,
) -> Result<Option<PrRow>, Error> {
    Ok(pull_requests::table
        .filter(pull_requests::repo_id.eq(repo_id))
        .filter(pull_requests::number.eq(number))
        .select(PrRow::as_select())
        .first(conn)
        .optional()?)
}

fn branch(r#ref: &str) -> Option<&str> {
    r#ref.strip_prefix("refs/heads/")
}

/// Before a branch is deleted on GitHub: GitHub closes every open pull
/// request based on it, and a closed pull request cannot be retargeted.
/// vcsstar already moved its PRs to the default branch (`prs::on_push`),
/// so move the GitHub ones the same way first.
pub fn prepare_branch_delete(
    conn: &mut PgConnection,
    repo: &Repo,
    state: &StateRow,
    gh: &github::Client,
    deleted_branch: &str,
) -> Result<usize, Failure> {
    let full = state.github_repo.as_str();
    let mut moved = 0;
    for j in gh.pulls_by_base(full, deleted_branch)? {
        let Some(n) = j.get("number").and_then(Json::as_i64) else {
            continue;
        };
        let Some(pr) =
            pr_by_github_number(conn, repo.id, n).map_err(|e| Failure::Transient(e.message))?
        else {
            continue;
        };
        let Some(base) = branch(&pr.base_ref) else {
            continue;
        };
        if pr.state != "open" || base == deleted_branch {
            continue;
        }
        gh.patch_pull(full, n, &json!({ "base": base }))?;
        moved += 1;
    }
    Ok(moved)
}

/// Converge the GitHub pull request of vcsstar PR `number`.
pub fn exec_pr_sync(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    number: i32,
) -> Result<Exec, Error> {
    let mut used = None;
    let r = finish(pr_sync(
        app, conn, org, repo, state, action, number, &mut used,
    ));
    settle(conn, &used, r)
}

#[allow(clippy::too_many_arguments)]
fn pr_sync(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    number: i32,
    used: &mut Option<Token>,
) -> Result<Exec, Stop> {
    let Some(pr) = load_pr_opt(conn, repo.id, number)? else {
        return Ok(Exec::Skipped("pull request is gone".into()));
    };
    if pr.head_remote.is_some() || pr.head_repo_id.is_some_and(|h| h != repo.id) {
        return Ok(Exec::Skipped(
            "a pull request from a fork is not created on GitHub".into(),
        ));
    }
    let (Some(head_branch), Some(base_branch)) = (branch(&pr.head_ref), branch(&pr.base_ref))
    else {
        return Ok(Exec::Skipped("head or base is not a branch".into()));
    };
    let full = state.github_repo.as_str();
    let author = prs::identity(conn, pr.author_identity_id)?;
    let name = display_name(&author);
    let url = pr_url(app, org, repo, pr.number);
    let gh = client(app, conn, org, repo, action, used)?;
    // Acting as the author needs no attribution line; the marker stays
    // so an early webhook links the pull request instead of duplicating it.
    let footer = if acted_as_user(used) {
        format!("\n\n{}", marker("pr", &pr.number.to_string()))
    } else {
        format!(
            "{FOOTER_START} #{}]({url}) opened by **{name}**._\n{}",
            pr.number,
            marker("pr", &pr.number.to_string())
        )
    };
    let gh_body = format!("{}{footer}", normalize(pr.body.as_deref().unwrap_or("")));
    let github_number: i64 = match pr.github_number {
        Some(n) => n as i64,
        None => {
            if pr.state != "open" {
                return Ok(Exec::Skipped(
                    "closed before it reached GitHub; nothing to create".into(),
                ));
            }
            if mirror::mirrored_sha(conn, repo.id, &pr.head_ref)?.as_deref()
                != Some(pr.head_sha.as_str())
            {
                return Ok(Exec::Transient(format!(
                    "head branch {head_branch} is not on GitHub at {} yet",
                    short(&pr.head_sha)
                )));
            }
            if mirror::mirrored_sha(conn, repo.id, &pr.base_ref)?.is_none() {
                return Ok(Exec::Transient(format!(
                    "base branch {base_branch} is not on GitHub yet"
                )));
            }
            let owner = full.split('/').next().unwrap_or("");
            let existing = gh.find_pulls(full, &format!("{owner}:{head_branch}"), base_branch)?;
            let (j, created) = match existing.into_iter().next() {
                Some(j) => (j, false),
                None => (
                    gh.create_pull(full, &pr.title, &gh_body, head_branch, base_branch)?,
                    true,
                ),
            };
            let n = j.get("number").and_then(Json::as_i64).ok_or_else(|| {
                Failure::Terminal("GitHub returned no pull request number".into())
            })?;
            let linked = diesel::update(pull_requests::table.find(pr.id))
                .set(pull_requests::github_number.eq(n as i32))
                .execute(conn);
            if let Err(e) = linked {
                return Ok(Exec::Blocked(format!(
                    "GitHub pull request #{n} is already linked to another vcsstar pull request: {e}"
                )));
            }
            if created {
                return Ok(Exec::Applied);
            }
            n
        }
    };
    let Some(current) = gh.get_pull(full, github_number)? else {
        return Ok(Exec::Blocked(format!(
            "GitHub pull request #{github_number} does not exist any more"
        )));
    };
    let s = |p: &str| current.pointer(p).and_then(Json::as_str).unwrap_or("");
    // Born on GitHub (no vcsstar marker in its body): GitHub keeps its
    // title and body; vcsstar's copy follows them on import.
    let github_owned = find_marker(s("/body"), "pr").is_none();
    let cur_merged = current
        .get("merged")
        .and_then(Json::as_bool)
        .unwrap_or(false);
    let cur_state = s("/state");
    let mut patch = json!({});
    if !github_owned {
        if s("/title") != pr.title {
            patch["title"] = json!(pr.title);
        }
        if normalize(s("/body")) != normalize(&gh_body) {
            patch["body"] = json!(gh_body);
        }
    }
    if pr.state == "open" && s("/base/ref") != base_branch {
        patch["base"] = json!(base_branch);
    }
    match pr.state.as_str() {
        "open" => {
            if cur_state != "open" {
                if cur_merged {
                    return Ok(Exec::Blocked(format!(
                        "GitHub pull request #{github_number} is merged there; reopen is impossible"
                    )));
                }
                patch["state"] = json!("open");
            }
        }
        "closed" => {
            if cur_state == "open" {
                patch["state"] = json!("closed");
            }
        }
        "merged" if !cur_merged => {
            {
                let Some(merged) = pr.merged_commit_sha.clone() else {
                    return Ok(Exec::Blocked("merged without a merged commit".into()));
                };
                let on_github = mirror::mirrored_sha(conn, repo.id, &pr.base_ref)?;
                let landed = match &on_github {
                    Some(sha) if *sha == merged => true,
                    Some(sha) => {
                        let dir = vcs::hydrate(app, conn, repo)?;
                        app.store
                            .backend()
                            .is_ancestor(
                                &dir,
                                &RevId(merged.clone()),
                                &RevId(sha.clone()),
                                &Env::new(),
                            )
                            .unwrap_or(false)
                    }
                    None => false,
                };
                if !landed {
                    return Ok(Exec::Transient(format!(
                        "merged commit {} is not on GitHub's {base_branch} yet",
                        short(&merged)
                    )));
                }
                // GitHub flips a merge-commit PR to merged by itself once the
                // push lands; look again before closing it by hand.
                let again = gh.get_pull(full, github_number)?;
                let merged_now = again
                    .as_ref()
                    .and_then(|j| j.get("merged"))
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                if merged_now {
                    return Ok(Exec::Applied);
                }
                // GitHub notices a merge commit a few seconds after the
                // push lands; give it two retries before closing by hand.
                if pr.merge_kind.as_deref() == Some("merge") && action.attempts < 3 {
                    return Ok(Exec::Transient(
                        "waiting for GitHub to detect the merge commit".into(),
                    ));
                }
                if cur_state == "open" {
                    let by = match pr.merged_by {
                        Some(id) => display_name(&prs::identity(conn, id)?),
                        None => "vcsstar".into(),
                    };
                    gh.create_issue_comment(
                        full,
                        github_number,
                        &format!(
                            "Merged on [vcsstar]({url}) by **{by}** as `{}` ({}).\n{}",
                            short(&merged),
                            pr.merge_kind.as_deref().unwrap_or("merge"),
                            marker("pr", &pr.number.to_string())
                        ),
                    )?;
                    patch["state"] = json!("closed");
                }
            }
        }
        _ => {}
    }
    // A closed pull request cannot change its base: reopen first.
    if patch.get("state") == Some(&json!("open")) && patch.get("base").is_some() {
        gh.patch_pull(full, github_number, &json!({ "state": "open" }))?;
        patch.as_object_mut().map(|m| m.remove("state"));
    }
    if patch.as_object().is_some_and(|m| !m.is_empty()) {
        gh.patch_pull(full, github_number, &patch)?;
    }
    Ok(Exec::Applied)
}

/// Post or edit one vcsstar comment on the GitHub pull request.
pub fn exec_pr_comment(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    id: Uuid,
) -> Result<Exec, Error> {
    let mut used = None;
    let r = finish(pr_comment(
        app, conn, org, repo, state, action, id, &mut used,
    ));
    settle(conn, &used, r)
}

#[allow(clippy::too_many_arguments)]
fn pr_comment(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    id: Uuid,
    used: &mut Option<Token>,
) -> Result<Exec, Stop> {
    let Some(c): Option<CommentRow> = pr_comments::table
        .find(id)
        .select(CommentRow::as_select())
        .first(conn)
        .optional()?
    else {
        return Ok(Exec::Skipped("comment is gone".into()));
    };
    let pr: PrRow = pull_requests::table
        .find(c.pr_id)
        .select(PrRow::as_select())
        .first(conn)?;
    if is_shadow_actor(conn, Some(c.author))? {
        return Ok(Exec::Skipped("GitHub's own comment".into()));
    }
    let Some(github_number) = pr.github_number.map(|n| n as i64) else {
        return Ok(Exec::Transient(
            "the pull request is not on GitHub yet".into(),
        ));
    };
    let full = state.github_repo.as_str();
    let author = prs::identity(conn, c.author)?;
    let url = pr_url(app, org, repo, pr.number);
    let gh = client(app, conn, org, repo, action, used)?;
    let as_user = acted_as_user(used);
    let gh_body = if as_user {
        format!(
            "{}\n{}",
            normalize(&c.body),
            marker("comment", &c.id.to_string())
        )
    } else {
        format!(
            "**{}** [commented on vcsstar]({url}):\n\n{}\n{}",
            display_name(&author),
            normalize(&c.body),
            marker("comment", &c.id.to_string())
        )
    };
    if let Some(gid) = c.github_id {
        if c.edited_at.is_some() {
            if c.github_kind.as_deref() == Some("review") {
                gh.patch_review_comment(full, gid, &gh_body)?;
            } else {
                gh.patch_issue_comment(full, gid, &gh_body)?;
            }
        }
        return Ok(Exec::Applied);
    }
    let store = |conn: &mut PgConnection, gid: i64, kind: &str| -> Result<(), Stop> {
        diesel::update(pr_comments::table.find(c.id))
            .set((
                pr_comments::github_id.eq(gid),
                pr_comments::github_kind.eq(kind),
            ))
            .execute(conn)?;
        Ok(())
    };
    if let (Some(path), Some(line), Some(sha)) = (&c.path, c.line, &c.commit_sha) {
        match gh.create_review_comment(full, github_number, &gh_body, sha, path, line as i64) {
            Ok(gid) => {
                store(conn, gid, "review")?;
                return Ok(Exec::Applied);
            }
            Err(Failure::Terminal(m)) if github::is_missing_commit(&m) => {
                return Ok(Exec::Transient(m));
            }
            Err(Failure::Terminal(_)) => {
                // Not a diff line GitHub knows: keep it as a conversation
                // comment that names the place.
                let located = if as_user {
                    format!(
                        "{}\n\n_(at `{path}:{line}`, {})_\n{}",
                        normalize(&c.body),
                        short(sha),
                        marker("comment", &c.id.to_string())
                    )
                } else {
                    format!(
                        "**{}** [commented on vcsstar]({url}) at `{path}:{line}` ({}):\n\n{}\n{}",
                        display_name(&author),
                        short(sha),
                        normalize(&c.body),
                        marker("comment", &c.id.to_string())
                    )
                };
                let gid = gh.create_issue_comment(full, github_number, &located)?;
                store(conn, gid, "issue")?;
                return Ok(Exec::Applied);
            }
            Err(f) => return Err(f.into()),
        }
    }
    let gid = gh.create_issue_comment(full, github_number, &gh_body)?;
    store(conn, gid, "issue")?;
    Ok(Exec::Applied)
}

/// Submit one vcsstar review on the GitHub pull request.
pub fn exec_pr_review(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    id: Uuid,
) -> Result<Exec, Error> {
    let mut used = None;
    let r = finish(pr_review(
        app, conn, org, repo, state, action, id, &mut used,
    ));
    settle(conn, &used, r)
}

#[allow(clippy::too_many_arguments)]
fn pr_review(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    id: Uuid,
    used: &mut Option<Token>,
) -> Result<Exec, Stop> {
    let Some(r): Option<ReviewRow> = pr_reviews::table
        .find(id)
        .select(ReviewRow::as_select())
        .first(conn)
        .optional()?
    else {
        return Ok(Exec::Skipped("review is gone".into()));
    };
    if r.github_id.is_some() {
        return Ok(Exec::Applied);
    }
    let pr: PrRow = pull_requests::table
        .find(r.pr_id)
        .select(PrRow::as_select())
        .first(conn)?;
    if is_shadow_actor(conn, Some(r.reviewer))? {
        return Ok(Exec::Skipped("GitHub's own review".into()));
    }
    let Some(github_number) = pr.github_number.map(|n| n as i64) else {
        return Ok(Exec::Transient(
            "the pull request is not on GitHub yet".into(),
        ));
    };
    let full = state.github_repo.as_str();
    let reviewer = prs::identity(conn, r.reviewer)?;
    let url = pr_url(app, org, repo, pr.number);
    let (event, verb) = match r.state.as_str() {
        "approved" => ("APPROVE", "approved"),
        "changes_requested" => ("REQUEST_CHANGES", "requested changes"),
        _ => ("COMMENT", "commented"),
    };
    let text = normalize(r.body.as_deref().unwrap_or(""));
    let gh = client(app, conn, org, repo, action, used)?;
    let gh_body = if acted_as_user(used) {
        format!("{text}\n{}", marker("review", &r.id.to_string()))
    } else {
        format!(
            "**{}** {verb} on [vcsstar]({url}) at `{}`.{}\n{}",
            display_name(&reviewer),
            short(&r.head_sha_at_review),
            if text.is_empty() {
                String::new()
            } else {
                format!("\n\n{text}")
            },
            marker("review", &r.id.to_string())
        )
    };
    let gid = match gh.create_review(full, github_number, event, &gh_body, &r.head_sha_at_review) {
        Ok(gid) => gid,
        Err(Failure::Terminal(m)) if github::is_self_review_refusal(&m) && event != "COMMENT" => {
            // The mirror credential also authored the GitHub pull request,
            // and GitHub refuses a self-review: keep the verdict visible.
            let noted = format!(
                "{gh_body}\n\n_(recorded as a comment: GitHub does not accept this verdict from the account that opened the pull request)_"
            );
            gh.create_review(
                full,
                github_number,
                "COMMENT",
                &noted,
                &r.head_sha_at_review,
            )?
        }
        Err(Failure::Terminal(m)) if github::is_missing_commit(&m) => {
            return Ok(Exec::Transient(m));
        }
        Err(f) => return Err(f.into()),
    };
    diesel::update(pr_reviews::table.find(r.id))
        .set(pr_reviews::github_id.eq(gid))
        .execute(conn)?;
    Ok(Exec::Applied)
}

/// Report one vcsstar commit status to GitHub.
#[allow(clippy::too_many_arguments)]
pub fn exec_status(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    sha: &str,
    context: &str,
) -> Result<Exec, Error> {
    let mut used = None;
    let r = finish(status(
        app, conn, org, repo, state, action, sha, context, &mut used,
    ));
    settle(conn, &used, r)
}

#[allow(clippy::too_many_arguments)]
fn status(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    state: &StateRow,
    action: &ActionRow,
    sha: &str,
    context: &str,
    used: &mut Option<Token>,
) -> Result<Exec, Stop> {
    let Some(row): Option<StatusRow> = statuses::table
        .filter(statuses::repo_id.eq(repo.id))
        .filter(statuses::sha.eq(sha))
        .filter(statuses::context.eq(context))
        .select(StatusRow::as_select())
        .first(conn)
        .optional()?
    else {
        return Ok(Exec::Skipped("status is gone".into()));
    };
    if is_shadow_actor(conn, row.reporter)? {
        return Ok(Exec::Skipped("GitHub's own status".into()));
    }
    let gh = client(app, conn, org, repo, action, used)?;
    match gh.create_status(
        &state.github_repo,
        &row.sha,
        &row.state,
        &row.context,
        row.target_url.as_deref(),
        row.description.as_deref(),
    ) {
        Ok(()) => Ok(Exec::Applied),
        Err(Failure::Terminal(m)) if github::is_missing_commit(&m) => Ok(Exec::Transient(m)),
        Err(f) => Err(f.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip_and_strip() {
        let id = Uuid::now_v7();
        let body = format!("hello\n{}\n", marker("comment", &id.to_string()));
        assert_eq!(
            find_marker(&body, "comment").as_deref(),
            Some(id.to_string().as_str())
        );
        assert_eq!(find_marker(&body, "review"), None);
        assert_eq!(strip_vcsstar(&body), "hello");
        let with_footer = format!(
            "A body.\r\n\r\nMore.{FOOTER_START} #4](https://x/o/r/pulls/4) opened by **a@b**._\n{}",
            marker("pr", "4")
        );
        assert_eq!(strip_vcsstar(&with_footer), "A body.\n\nMore.");
        assert_eq!(find_marker(&with_footer, "pr").as_deref(), Some("4"));
    }

    #[test]
    fn pull_payload_maps_states() {
        let j = json!({
            "number": 7, "title": "T", "body": null, "state": "closed",
            "merged": true, "merge_commit_sha": "abc", "merged_by": {"id": 1, "login": "m"},
            "user": {"id": 2, "login": "u"},
            "head": {"sha": "h", "ref": "feat", "repo": {"full_name": "o/r"}},
            "base": {"sha": "b", "ref": "main"},
        });
        let p = parse_pull(&j).unwrap();
        assert_eq!(p.state, "merged");
        assert_eq!(p.head_repo.as_deref(), Some("o/r"));
        assert_eq!(p.body, "");
        let mut open = j.clone();
        open["state"] = json!("open");
        open["merged"] = json!(false);
        assert_eq!(parse_pull(&open).unwrap().state, "open");
        let mut closed = open.clone();
        closed["state"] = json!("closed");
        assert_eq!(parse_pull(&closed).unwrap().state, "closed");
    }

    #[test]
    fn check_runs_become_statuses() {
        assert_eq!(check_run_state("queued", None), "pending");
        assert_eq!(check_run_state("in_progress", None), "pending");
        assert_eq!(check_run_state("completed", Some("success")), "success");
        assert_eq!(check_run_state("completed", Some("skipped")), "success");
        assert_eq!(check_run_state("completed", Some("failure")), "failure");
        assert_eq!(check_run_state("completed", Some("cancelled")), "failure");
        assert_eq!(
            check_run_state("completed", Some("action_required")),
            "error"
        );
    }

    #[test]
    fn shadow_ids_are_stable() {
        assert_eq!(shadow_user_id(42), shadow_user_id(42));
        assert_ne!(shadow_user_id(42), shadow_user_id(43));
    }
}
