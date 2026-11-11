//! Pull requests. A PR proposes moving a base ref to include the history
//! of a head ref. Internal refs `refs/vcsstar/pr/{n}/head` and `/merge`
//! live in `repo_refs` like any ref and are excluded from listings and
//! mirroring. On a mirrored repository the PR is synced with a GitHub pull
//! request both ways (`github_sync`).

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use vcsstar_db::models::{Identity, Org as OrgRow, Repo as RepoRow};
use vcsstar_db::schema::{identities, pr_comments, pr_counters, pr_reviews, pull_requests, repos};
use vcsstar_db::DbConn;
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_types::{
    Comment, CommentRequest, EventPayload, ListCommentsResponse, ListPrRequest,
    ListReviewsResponse, MergeRequest, MergeResult, OpenPrRequest, PrHead, PrPayload, PrRef,
    PullRequest, RetargetRequest, Review, ReviewRequest, UpdatePrRequest,
};
use vcsstar_vcs::{CommitMsg, Env, MergeKind, MergeOutcome, RevId};

use crate::authz::{self, RepoRole};
use crate::outbox::{self, Emit};
use crate::principal::{identity_to_wire, Principal, ReqCtx};
use crate::services::{keys::AllowedSigners, repos as repo_svc, statuses};
use crate::{instance_keys, push, to_ms, vcs, App, Code, Error};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = pull_requests)]
pub struct PrRow {
    pub id: uuid::Uuid,
    pub repo_id: uuid::Uuid,
    pub number: i32,
    pub title: String,
    pub body: Option<String>,
    pub author_identity_id: uuid::Uuid,
    pub head_repo_id: Option<uuid::Uuid>,
    pub head_remote: Option<String>,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub base_sha: String,
    pub merge_base_sha: Option<String>,
    pub mergeable: String,
    pub state: String,
    pub merge_kind: Option<String>,
    pub merged_commit_sha: Option<String>,
    pub merged_by: Option<uuid::Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub github_number: Option<i32>,
    /// GitHub's `updated_at` of the pull request as last applied here.
    pub github_updated_at: Option<DateTime<Utc>>,
}

pub fn head_ref_name(n: i32) -> String {
    format!("refs/vcsstar/pr/{n}/head")
}
pub fn merge_ref_name(n: i32) -> String {
    format!("refs/vcsstar/pr/{n}/merge")
}

pub(crate) fn full_ref(s: &str) -> String {
    if s.starts_with("refs/") {
        s.to_string()
    } else {
        format!("refs/heads/{s}")
    }
}

pub(crate) fn identity(
    conn: &mut PgConnection,
    id: uuid::Uuid,
) -> Result<vcsstar_types::Identity, Error> {
    let i: Option<Identity> = identities::table
        .find(id)
        .select(Identity::as_select())
        .first(conn)
        .optional()?;
    Ok(i.map(|i| identity_to_wire(&i))
        .unwrap_or(vcsstar_types::Identity {
            user_id: String::new(),
            domain: String::new(),
            handle: None,
            display_name: None,
        }))
}

pub(crate) fn to_wire(
    conn: &mut PgConnection,
    org: &OrgRow,
    repo: &RepoRow,
    pr: &PrRow,
) -> Result<PullRequest, Error> {
    let head_repo = match pr.head_repo_id {
        Some(id) if id != repo.id => {
            let (r, o): (RepoRow, OrgRow) = repos::table
                .inner_join(vcsstar_db::schema::orgs::table)
                .filter(repos::id.eq(id))
                .select((RepoRow::as_select(), OrgRow::as_select()))
                .first(conn)?;
            vcs::repo_ref(&o, &r)
        }
        _ => vcsstar_types::RepoRef {
            instance: pr.head_remote.clone(),
            org: org.name.clone(),
            name: repo.name.clone(),
        },
    };
    let merged_by = match pr.merged_by {
        Some(id) => Some(identity(conn, id)?),
        None => None,
    };
    Ok(PullRequest {
        repo: vcs::repo_ref(org, repo),
        number: pr.number.max(0) as u64,
        title: pr.title.clone(),
        body: pr.body.clone(),
        author: identity(conn, pr.author_identity_id)?,
        state: pr.state.clone(),
        base_ref: pr.base_ref.clone(),
        head: PrHead {
            repo: head_repo,
            r#ref: pr.head_ref.clone(),
        },
        base_sha: pr.base_sha.clone(),
        head_sha: pr.head_sha.clone(),
        merge_base_sha: pr.merge_base_sha.clone(),
        mergeable: pr.mergeable.clone(),
        merge_kind: pr.merge_kind.clone(),
        merged_commit: pr.merged_commit_sha.clone(),
        merged_by,
        created_at: to_ms(pr.created_at),
        updated_at: to_ms(pr.updated_at),
        closed_at: pr.closed_at.map(to_ms),
        github_number: pr.github_number.map(|n| n.max(0) as u64),
        github_url: match pr.github_number {
            Some(n) => crate::mirror::state_for(conn, repo.id)?
                .map(|s| format!("https://github.com/{}/pull/{n}", s.github_repo)),
            None => None,
        },
    })
}

pub(crate) fn load_pr(
    conn: &mut PgConnection,
    repo_id: uuid::Uuid,
    number: u64,
) -> Result<PrRow, Error> {
    pull_requests::table
        .filter(pull_requests::repo_id.eq(repo_id))
        .filter(pull_requests::number.eq(number.min(i32::MAX as u64) as i32))
        .select(PrRow::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("pull request"))
}

pub(crate) fn pr_event<'a>(
    org: &OrgRow,
    repo: &RepoRow,
    actor: Option<&Principal>,
    kind: &'a str,
    pr: &PrRow,
) -> Emit<'a> {
    pr_event_about(org, repo, actor, kind, pr, None, None)
}

/// A PR event that names the comment or review row it is about.
pub(crate) fn pr_event_about<'a>(
    org: &OrgRow,
    repo: &RepoRow,
    actor: Option<&Principal>,
    kind: &'a str,
    pr: &PrRow,
    comment_id: Option<uuid::Uuid>,
    review_id: Option<uuid::Uuid>,
) -> Emit<'a> {
    Emit {
        lane: outbox::lane_for_repo(&org.name, &repo.name),
        kind,
        repo_id: Some(repo.id),
        org_id: Some(org.id),
        actor_id: actor.map(|p| p.identity.id),
        actor: actor.map(|p| p.to_wire()),
        repo: Some(vcs::repo_ref(org, repo)),
        org: Some(org.name.clone()),
        subject: Some(pr.number.to_string()),
        payload: EventPayload::Variant2(PrPayload {
            number: pr.number.max(0) as u64,
            head_sha: Some(pr.head_sha.clone()),
            base_ref: Some(pr.base_ref.clone()),
            state: Some(pr.state.clone()),
            merge_kind: pr.merge_kind.clone(),
            comment_id: comment_id.map(|id| id.to_string()),
            review_id: review_id.map(|id| id.to_string()),
        }),
    }
}

/// Bring the head commit into the base repo's cache (fork PRs) and record
/// the objects as a base-repo segment.
pub(crate) fn snapshot_head(
    app: &App,
    conn: &mut PgConnection,
    base_repo: &RepoRow,
    base_dir: &std::path::Path,
    head_repo: &RepoRow,
    head_sha: &RevId,
) -> Result<(), Error> {
    let backend = app.store.backend();
    if backend.object_exists(base_dir, head_sha, &Env::new()) {
        return Ok(());
    }
    if head_repo.id != base_repo.id {
        let head_dir = app.store.repo_dir(head_repo.id);
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(base_dir)
            .args(["fetch", "--quiet", "--no-tags"])
            .arg(&head_dir)
            .arg(&head_sha.0)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .map_err(Error::internal)?;
        if !out.status.success() {
            return Err(Error::new(
                Code::PreconditionFailed,
                format!(
                    "cannot fetch head {head_sha} from the fork: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
        vcs::capture(
            app,
            conn,
            base_repo.id,
            base_dir,
            &[],
            std::slice::from_ref(head_sha),
        )?;
    }
    if !backend.object_exists(base_dir, head_sha, &Env::new()) {
        return Err(Error::new(
            Code::PreconditionFailed,
            format!("head {head_sha} is not available"),
        ));
    }
    Ok(())
}

/// Trial merge: records `mergeable`, `merge_base_sha`, and the `/merge`
/// internal ref. Nothing is written to the base ref.
pub(crate) fn trial_merge(
    app: &App,
    conn: &mut PgConnection,
    repo: &RepoRow,
    dir: &std::path::Path,
    pr: &PrRow,
) -> Result<PrRow, Error> {
    let backend = app.store.backend();
    let base = RevId(pr.base_sha.clone());
    let head = RevId(pr.head_sha.clone());
    let merge_base = backend.common_ancestor(dir, &base, &head)?;
    let msg = CommitMsg {
        message: format!("Trial merge of pull request #{}", pr.number),
        author_name: "vcsstar".into(),
        author_email: format!("vcsstar@{}", app.config.instance_domain),
        committer_name: "vcsstar".into(),
        committer_email: format!("vcsstar@{}", app.config.instance_domain),
    };
    let (mergeable, merge_commit) = if backend
        .is_ancestor(dir, &head, &base, &Env::new())
        .unwrap_or(false)
    {
        ("clean", None) // already merged into base
    } else {
        match backend.merge(dir, MergeKind::Merge, &base, &head, &msg, None, &[])? {
            MergeOutcome::Clean(id) => ("clean", Some(id)),
            MergeOutcome::Conflicts(_) => ("conflicts", None),
        }
    };
    let merge_ref = merge_ref_name(pr.number);
    let current = refs::get(conn, repo.id, &merge_ref)?;
    if current != merge_commit {
        if let Some(c) = &merge_commit {
            vcs::capture(
                app,
                conn,
                repo.id,
                dir,
                &[base.clone(), head.clone()],
                std::slice::from_ref(c),
            )?;
        }
        refs::apply(
            conn,
            repo.id,
            &[RefUpdate {
                name: merge_ref,
                old: current,
                new: merge_commit.clone(),
            }],
            None,
        )?;
    }
    let updated: PrRow = diesel::update(pull_requests::table.find(pr.id))
        .set((
            pull_requests::mergeable.eq(mergeable),
            pull_requests::merge_base_sha.eq(merge_base.map(|r| r.0)),
            pull_requests::updated_at.eq(Utc::now()),
        ))
        .returning(PrRow::as_returning())
        .get_result(conn)?;
    Ok(updated)
}

pub fn open(app: &App, ctx: &ReqCtx, req: OpenPrRequest) -> Result<PullRequest, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:write")?;
    if req.title.trim().is_empty() || req.title.len() > 300 {
        return Err(Error::invalid("title is required (max 300 chars)"));
    }
    if req.head.repo.instance.is_some() {
        return Err(Error::not_implemented(
            "cross-instance pull requests (phase 6)",
        ));
    }
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Read)?;
    let (head_org, head_repo, head_role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.head.repo.org, &req.head.repo.name)?;
    authz::require_repo_role(head_role, RepoRole::Read)?;
    if head_repo.id != repo.id && head_repo.fork_of_repo_id != Some(repo.id) {
        return Err(Error::invalid(
            "the head repository must be this repository or a fork of it",
        ));
    }
    let base_ref = full_ref(&req.base_ref);
    let head_ref = full_ref(&req.head.r#ref);
    if head_repo.id == repo.id && base_ref == head_ref {
        return Err(Error::invalid("head and base are the same ref"));
    }
    let base_sha = refs::get(&mut conn, repo.id, &base_ref)?
        .ok_or_else(|| Error::not_found(&format!("base ref {base_ref}")))?;
    let head_sha = refs::get(&mut conn, head_repo.id, &head_ref)?
        .ok_or_else(|| Error::not_found(&format!("head ref {head_ref}")))?;
    let dir = vcs::hydrate(app, &mut conn, &repo)?;
    if head_repo.id != repo.id {
        vcs::hydrate(app, &mut conn, &head_repo)?;
    }
    let _ = head_org;
    let pr = conn.transaction::<PrRow, Error, _>(|conn| {
        snapshot_head(app, conn, &repo, &dir, &head_repo, &head_sha)?;
        let number: i32 = diesel::insert_into(pr_counters::table)
            .values((
                pr_counters::repo_id.eq(repo.id),
                pr_counters::next_number.eq(2),
            ))
            .on_conflict(pr_counters::repo_id)
            .do_update()
            .set(pr_counters::next_number.eq(pr_counters::next_number + 1))
            .returning(pr_counters::next_number)
            .get_result::<i32>(conn)?
            - 1;
        let pr: PrRow = diesel::insert_into(pull_requests::table)
            .values((
                pull_requests::id.eq(vcsstar_db::new_id()),
                pull_requests::repo_id.eq(repo.id),
                pull_requests::number.eq(number),
                pull_requests::title.eq(req.title.trim()),
                pull_requests::body.eq(req.body.as_deref()),
                pull_requests::author_identity_id.eq(p.identity.id),
                pull_requests::head_repo_id.eq(head_repo.id),
                pull_requests::head_ref.eq(&head_ref),
                pull_requests::base_ref.eq(&base_ref),
                pull_requests::head_sha.eq(&head_sha.0),
                pull_requests::base_sha.eq(&base_sha.0),
            ))
            .returning(PrRow::as_returning())
            .get_result(conn)?;
        refs::apply(
            conn,
            repo.id,
            &[RefUpdate {
                name: head_ref_name(number),
                old: None,
                new: Some(head_sha.clone()),
            }],
            Some(p.identity.id),
        )?;
        let pr = trial_merge(app, conn, &repo, &dir, &pr)?;
        outbox::emit(conn, pr_event(&org, &repo, Some(p), "pr_opened", &pr))?;
        Ok(pr)
    })?;
    to_wire(&mut conn, &org, &repo, &pr)
}

fn open_pr(
    app: &App,
    ctx: &ReqCtx,
    repo_ref: &vcsstar_types::RepoRef,
    number: u64,
    min: RepoRole,
) -> Result<(DbConn, OrgRow, RepoRow, PrRow, RepoRole), Error> {
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &repo_ref.org, &repo_ref.name)?;
    authz::require_repo_role(role, min)?;
    let pr = load_pr(&mut conn, repo.id, number)?;
    Ok((conn, org, repo, pr, role))
}

pub fn get(app: &App, ctx: &ReqCtx, req: PrRef) -> Result<PullRequest, Error> {
    let (mut conn, org, repo, mut pr, _) =
        open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    if pr.state == "open" && pr.mergeable == "unknown" {
        let dir = vcs::hydrate(app, &mut conn, &repo)?;
        pr =
            conn.transaction::<PrRow, Error, _>(|conn| trial_merge(app, conn, &repo, &dir, &pr))?;
    }
    to_wire(&mut conn, &org, &repo, &pr)
}

pub fn list(app: &App, ctx: &ReqCtx, req: ListPrRequest) -> Result<vcsstar_types::PrPage, Error> {
    let mut conn = app.conn()?;
    let (org, repo, _role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(30)
        .clamp(1, 200) as i64;
    let mut q = pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .into_boxed();
    match req.state.as_deref() {
        None | Some("open") => q = q.filter(pull_requests::state.eq("open")),
        Some("all") => {}
        Some(s) => q = q.filter(pull_requests::state.eq(s)),
    }
    if let Some(c) = req.page.as_ref().and_then(|p| p.cursor.as_deref()) {
        let n: i32 = c.parse().map_err(|_| Error::invalid("cursor"))?;
        q = q.filter(pull_requests::number.lt(n));
    }
    let rows: Vec<PrRow> = q
        .order(pull_requests::number.desc())
        .limit(limit + 1)
        .select(PrRow::as_select())
        .load(&mut conn)?;
    let next_cursor =
        (rows.len() as i64 > limit).then(|| rows[limit as usize - 1].number.to_string());
    let mut out = Vec::new();
    for r in rows.iter().take(limit as usize) {
        out.push(to_wire(&mut conn, &org, &repo, r)?);
    }
    Ok(vcsstar_types::PrPage {
        pull_requests: out,
        next_cursor,
    })
}

pub fn update(app: &App, ctx: &ReqCtx, req: UpdatePrRequest) -> Result<PullRequest, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:write")?;
    let (mut conn, org, repo, pr, role) = open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    if pr.author_identity_id != p.identity.id && role < RepoRole::Maintain {
        return Err(Error::forbidden("only the author or a maintainer may edit"));
    }
    if let Some(t) = &req.title {
        if t.trim().is_empty() || t.len() > 300 {
            return Err(Error::invalid("title is required (max 300 chars)"));
        }
    }
    let updated = conn.transaction::<PrRow, Error, _>(|conn| {
        let u: PrRow = diesel::update(pull_requests::table.find(pr.id))
            .set((
                req.title
                    .as_ref()
                    .map(|t| pull_requests::title.eq(t.trim().to_string())),
                req.body.as_ref().map(|b| pull_requests::body.eq(b)),
                pull_requests::updated_at.eq(Utc::now()),
            ))
            .returning(PrRow::as_returning())
            .get_result(conn)?;
        outbox::emit(conn, pr_event(&org, &repo, Some(p), "pr_updated", &u))?;
        Ok(u)
    })?;
    to_wire(&mut conn, &org, &repo, &updated)
}

fn set_state(
    app: &App,
    ctx: &ReqCtx,
    req: PrRef,
    from: &str,
    to: &str,
    kind: &'static str,
) -> Result<PullRequest, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:write")?;
    let (mut conn, org, repo, pr, role) = open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    if pr.author_identity_id != p.identity.id && role < RepoRole::Write {
        return Err(Error::forbidden(
            "only the author or a writer may change the state",
        ));
    }
    if pr.state != from {
        return Err(Error::new(
            Code::PreconditionFailed,
            format!("pull request is {}", pr.state),
        ));
    }
    let updated = conn.transaction::<PrRow, Error, _>(|conn| {
        let u: PrRow = diesel::update(pull_requests::table.find(pr.id))
            .set((
                pull_requests::state.eq(to),
                pull_requests::closed_at.eq(if to == "closed" {
                    Some(Utc::now())
                } else {
                    None
                }),
                pull_requests::mergeable.eq(if to == "open" {
                    "unknown"
                } else {
                    pr.mergeable.as_str()
                }),
                pull_requests::updated_at.eq(Utc::now()),
            ))
            .returning(PrRow::as_returning())
            .get_result(conn)?;
        if to == "closed" {
            drop_internal_refs(conn, repo.id, pr.number)?;
        } else if to == "open" {
            // Reopen: materialize the head snapshot again.
            let head_sha = refs::get(conn, pr.head_repo_id.unwrap_or(repo.id), &pr.head_ref)?
                .map(|r| r.0)
                .unwrap_or(pr.head_sha.clone());
            let current_head = refs::get(conn, repo.id, &head_ref_name(pr.number))?;
            refs::apply(
                conn,
                repo.id,
                &[RefUpdate {
                    name: head_ref_name(pr.number),
                    old: current_head,
                    new: Some(RevId(head_sha.clone())),
                }],
                Some(p.identity.id),
            )?;
            diesel::update(pull_requests::table.find(pr.id))
                .set(pull_requests::head_sha.eq(&head_sha))
                .execute(conn)?;
        }
        outbox::emit(conn, pr_event(&org, &repo, Some(p), kind, &u))?;
        Ok(u)
    })?;
    to_wire(&mut conn, &org, &repo, &updated)
}

pub fn close(app: &App, ctx: &ReqCtx, req: PrRef) -> Result<PullRequest, Error> {
    set_state(app, ctx, req, "open", "closed", "pr_closed")
}

pub fn reopen(app: &App, ctx: &ReqCtx, req: PrRef) -> Result<PullRequest, Error> {
    set_state(app, ctx, req, "closed", "open", "pr_reopened")
}

pub(crate) fn drop_internal_refs(
    conn: &mut PgConnection,
    repo_id: uuid::Uuid,
    number: i32,
) -> Result<(), Error> {
    for name in [head_ref_name(number), merge_ref_name(number)] {
        if let Some(cur) = refs::get(conn, repo_id, &name)? {
            refs::apply(
                conn,
                repo_id,
                &[RefUpdate {
                    name,
                    old: Some(cur),
                    new: None,
                }],
                None,
            )?;
        }
    }
    Ok(())
}

pub fn retarget(app: &App, ctx: &ReqCtx, req: RetargetRequest) -> Result<PullRequest, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:write")?;
    let (mut conn, org, repo, pr, role) = open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    if pr.author_identity_id != p.identity.id && role < RepoRole::Write {
        return Err(Error::forbidden("only the author or a writer may retarget"));
    }
    if pr.state != "open" {
        return Err(Error::new(
            Code::PreconditionFailed,
            "pull request is not open",
        ));
    }
    let base_ref = full_ref(&req.base_ref);
    let base_sha = refs::get(&mut conn, repo.id, &base_ref)?
        .ok_or_else(|| Error::not_found(&format!("base ref {base_ref}")))?;
    let dir = vcs::hydrate(app, &mut conn, &repo)?;
    let updated = conn.transaction::<PrRow, Error, _>(|conn| {
        let u: PrRow = diesel::update(pull_requests::table.find(pr.id))
            .set((
                pull_requests::base_ref.eq(&base_ref),
                pull_requests::base_sha.eq(&base_sha.0),
                pull_requests::mergeable.eq("unknown"),
                pull_requests::updated_at.eq(Utc::now()),
            ))
            .returning(PrRow::as_returning())
            .get_result(conn)?;
        let u = trial_merge(app, conn, &repo, &dir, &u)?;
        outbox::emit(conn, pr_event(&org, &repo, Some(p), "pr_retargeted", &u))?;
        Ok(u)
    })?;
    to_wire(&mut conn, &org, &repo, &updated)
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = pr_reviews)]
pub(crate) struct ReviewRow {
    pub id: uuid::Uuid,
    pub pr_id: uuid::Uuid,
    pub reviewer: uuid::Uuid,
    pub state: String,
    pub head_sha_at_review: String,
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
    pub github_id: Option<i64>,
}

pub(crate) fn review_wire(
    conn: &mut PgConnection,
    pr: &PrRow,
    r: &ReviewRow,
) -> Result<Review, Error> {
    let _ = r.pr_id;
    Ok(Review {
        number: pr.number.max(0) as u64,
        reviewer: identity(conn, r.reviewer)?,
        state: r.state.clone(),
        head_sha: r.head_sha_at_review.clone(),
        body: r.body.clone(),
        created_at: to_ms(r.created_at),
    })
}

pub fn review(app: &App, ctx: &ReqCtx, req: ReviewRequest) -> Result<Review, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:review")?;
    if !["approved", "changes_requested", "comment"].contains(&req.state.as_str()) {
        return Err(Error::invalid(
            "state must be approved, changes_requested, or comment",
        ));
    }
    let (mut conn, org, repo, pr, _role) =
        open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    if pr.state != "open" {
        return Err(Error::new(
            Code::PreconditionFailed,
            "pull request is not open",
        ));
    }
    if req.head_sha != pr.head_sha {
        return Err(Error::new(
            Code::Conflict,
            format!(
                "the head moved: reviewed {} but the head is {}",
                req.head_sha, pr.head_sha
            ),
        ));
    }
    if req.state == "approved" && pr.author_identity_id == p.identity.id {
        return Err(Error::forbidden(
            "an author cannot approve their own pull request",
        ));
    }
    let row = conn.transaction::<ReviewRow, Error, _>(|conn| {
        let row: ReviewRow = diesel::insert_into(pr_reviews::table)
            .values((
                pr_reviews::id.eq(vcsstar_db::new_id()),
                pr_reviews::pr_id.eq(pr.id),
                pr_reviews::reviewer.eq(p.identity.id),
                pr_reviews::state.eq(&req.state),
                pr_reviews::head_sha_at_review.eq(&req.head_sha),
                pr_reviews::body.eq(req.body.as_deref()),
            ))
            .returning(ReviewRow::as_returning())
            .get_result(conn)?;
        diesel::update(pull_requests::table.find(pr.id))
            .set(pull_requests::updated_at.eq(Utc::now()))
            .execute(conn)?;
        outbox::emit(
            conn,
            pr_event_about(&org, &repo, Some(p), "pr_review", &pr, None, Some(row.id)),
        )?;
        Ok(row)
    })?;
    review_wire(&mut conn, &pr, &row)
}

pub fn list_reviews(app: &App, ctx: &ReqCtx, req: PrRef) -> Result<ListReviewsResponse, Error> {
    let (mut conn, _org, _repo, pr, _) = open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    let rows: Vec<ReviewRow> = pr_reviews::table
        .filter(pr_reviews::pr_id.eq(pr.id))
        .order(pr_reviews::created_at.asc())
        .select(ReviewRow::as_select())
        .load(&mut conn)?;
    let mut reviews = Vec::new();
    for r in &rows {
        reviews.push(review_wire(&mut conn, &pr, r)?);
    }
    Ok(ListReviewsResponse { reviews })
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = pr_comments)]
pub(crate) struct CommentRow {
    pub id: uuid::Uuid,
    pub pr_id: uuid::Uuid,
    pub author: uuid::Uuid,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<i32>,
    pub commit_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub github_id: Option<i64>,
    pub github_kind: Option<String>,
    /// GitHub's `updated_at` of the comment as last applied here.
    pub github_updated_at: Option<DateTime<Utc>>,
}

pub(crate) fn comment_wire(
    conn: &mut PgConnection,
    pr: &PrRow,
    c: &CommentRow,
) -> Result<Comment, Error> {
    let _ = c.pr_id;
    Ok(Comment {
        id: c.id.to_string(),
        number: pr.number.max(0) as u64,
        author: identity(conn, c.author)?,
        body: c.body.clone(),
        path: c.path.clone(),
        line: c.line.map(|l| l.max(0) as u64),
        commit_sha: c.commit_sha.clone(),
        created_at: to_ms(c.created_at),
        edited_at: c.edited_at.map(to_ms),
    })
}

pub fn comment(app: &App, ctx: &ReqCtx, req: CommentRequest) -> Result<Comment, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:write")?;
    if req.body.trim().is_empty() || req.body.len() > 65536 {
        return Err(Error::invalid("body is required (max 64 KiB)"));
    }
    let (mut conn, org, repo, pr, _) = open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    let row = conn.transaction::<CommentRow, Error, _>(|conn| {
        let row: CommentRow = diesel::insert_into(pr_comments::table)
            .values((
                pr_comments::id.eq(vcsstar_db::new_id()),
                pr_comments::pr_id.eq(pr.id),
                pr_comments::author.eq(p.identity.id),
                pr_comments::body.eq(&req.body),
                pr_comments::path.eq(req.path.as_deref()),
                pr_comments::line.eq(req.line.map(|l| l.min(i32::MAX as u64) as i32)),
                pr_comments::commit_sha.eq(req.commit_sha.as_deref()),
            ))
            .returning(CommentRow::as_returning())
            .get_result(conn)?;
        diesel::update(pull_requests::table.find(pr.id))
            .set(pull_requests::updated_at.eq(Utc::now()))
            .execute(conn)?;
        outbox::emit(
            conn,
            pr_event_about(&org, &repo, Some(p), "pr_comment", &pr, Some(row.id), None),
        )?;
        Ok(row)
    })?;
    comment_wire(&mut conn, &pr, &row)
}

pub fn list_comments(app: &App, ctx: &ReqCtx, req: PrRef) -> Result<ListCommentsResponse, Error> {
    let (mut conn, _org, _repo, pr, _) = open_pr(app, ctx, &req.repo, req.number, RepoRole::Read)?;
    let rows: Vec<CommentRow> = pr_comments::table
        .filter(pr_comments::pr_id.eq(pr.id))
        .order(pr_comments::created_at.asc())
        .select(CommentRow::as_select())
        .load(&mut conn)?;
    let mut comments = Vec::new();
    for c in &rows {
        comments.push(comment_wire(&mut conn, &pr, c)?);
    }
    Ok(ListCommentsResponse { comments })
}

/// The merge gates of plan/04, in order.
fn check_gates(
    app: &App,
    conn: &mut DbConn,
    repo: &RepoRow,
    dir: &std::path::Path,
    pr: &PrRow,
    kind: MergeKind,
) -> Result<(), Error> {
    let protections = repo_svc::protections_for(conn, repo.id)?;
    let matching: Vec<_> = protections
        .iter()
        .filter(|p| vcsstar_vcs::ref_pattern_matches(&p.ref_pattern, &pr.base_ref))
        .collect();
    let caps = app.store.backend().caps();
    let cap_ok = match kind {
        MergeKind::Merge => caps.merge_commit,
        MergeKind::Squash => caps.squash,
        MergeKind::Rebase => caps.rebase,
    };
    if !cap_ok {
        return Err(Error::new(
            Code::PreconditionFailed,
            format!("{} is not supported by this backend", kind.as_str()),
        ));
    }
    for pr_rule in &matching {
        let allowed: Vec<&str> = pr_rule
            .allowed_merge_kinds
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if !allowed.is_empty() && !allowed.contains(&kind.as_str()) {
            return Err(Error::new(
                Code::PreconditionFailed,
                format!(
                    "merge kind {} is not allowed on {}; allowed: {:?}",
                    kind.as_str(),
                    pr.base_ref,
                    allowed
                ),
            ));
        }
        if pr_rule.required_approvals > 0 {
            let mut q = pr_reviews::table
                .filter(pr_reviews::pr_id.eq(pr.id))
                .filter(pr_reviews::state.eq("approved"))
                .into_boxed();
            if pr_rule.dismiss_stale {
                q = q.filter(pr_reviews::head_sha_at_review.eq(&pr.head_sha));
            }
            let approvers: Vec<uuid::Uuid> =
                q.select(pr_reviews::reviewer).distinct().load(conn)?;
            if (approvers.len() as i32) < pr_rule.required_approvals {
                return Err(Error::new(
                    Code::PreconditionFailed,
                    format!(
                        "{} of {} required approvals",
                        approvers.len(),
                        pr_rule.required_approvals
                    ),
                ));
            }
        }
        let required: Vec<&str> = pr_rule
            .required_checks
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if !required.is_empty() {
            let have = statuses::for_sha(conn, repo.id, &pr.head_sha)?;
            for ctx_name in required {
                match have.iter().find(|s| s.context == ctx_name) {
                    Some(s) if s.state == "success" => {}
                    Some(s) => {
                        return Err(Error::new(
                            Code::PreconditionFailed,
                            format!("required check {ctx_name} is {}", s.state),
                        ))
                    }
                    None => {
                        return Err(Error::new(
                            Code::PreconditionFailed,
                            format!(
                                "required check {ctx_name} has not reported for {}",
                                pr.head_sha
                            ),
                        ))
                    }
                }
            }
        }
        if pr_rule.required_signature_level != "none" {
            let backend = app.store.backend();
            let commits = backend.rev_list(
                dir,
                Some(&RevId(pr.base_sha.clone())),
                &RevId(pr.head_sha.clone()),
                1000,
                &Env::new(),
            )?;
            let signers = AllowedSigners::build(conn)?;
            for c in &commits {
                let sig = signers.classify(conn, backend, dir, c, &Env::new())?;
                let ok = match pr_rule.required_signature_level.as_str() {
                    "attested" => sig.status == "verified",
                    _ => sig.status == "verified" || sig.status == "valid_unattested",
                };
                if !ok {
                    return Err(Error::new(
                        Code::PreconditionFailed,
                        format!(
                            "commit {} signature is {} (base requires {})",
                            c.id, sig.status, pr_rule.required_signature_level
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn merge(app: &App, ctx: &ReqCtx, req: MergeRequest) -> Result<MergeResult, Error> {
    let p = ctx.require()?;
    p.require_scope("pr:write")?;
    p.require_scope("repo:write")?;
    let kind = MergeKind::parse(&req.merge_kind)
        .ok_or_else(|| Error::invalid("merge_kind must be merge, squash, or rebase"))?;
    let (mut conn, org, repo, pr, role) =
        open_pr(app, ctx, &req.repo, req.number, RepoRole::Write)?;
    let _ = role;
    if pr.state != "open" {
        return Err(Error::new(
            Code::PreconditionFailed,
            format!("pull request is {}", pr.state),
        ));
    }
    if req.expected_head_sha != pr.head_sha {
        return Err(Error::new(
            Code::Conflict,
            format!(
                "stale view: expected head {} but it is {}",
                req.expected_head_sha, pr.head_sha
            ),
        ));
    }
    if repo.archived {
        return Err(Error::forbidden("repository is archived"));
    }
    let dir = vcs::hydrate(app, &mut conn, &repo)?;
    // Base may have moved since the snapshot: merge against the current tip.
    let base_now = refs::get(&mut conn, repo.id, &pr.base_ref)?
        .ok_or_else(|| Error::not_found(&format!("base ref {}", pr.base_ref)))?;
    let pr = if base_now.0 != pr.base_sha {
        diesel::update(pull_requests::table.find(pr.id))
            .set((
                pull_requests::base_sha.eq(&base_now.0),
                pull_requests::mergeable.eq("unknown"),
            ))
            .returning(PrRow::as_returning())
            .get_result::<PrRow>(&mut conn)?
    } else {
        pr
    };
    check_gates(app, &mut conn, &repo, &dir, &pr, kind)?;
    let author: Identity = identities::table
        .find(pr.author_identity_id)
        .select(Identity::as_select())
        .first(&mut conn)?;
    let author_email = match (&author.email, author.email_verified) {
        (Some(e), true) => e.clone(),
        _ => format!(
            "noreply+{}@{}",
            author.lk_user_id, app.config.instance_domain
        ),
    };
    let author_name = author
        .display_name
        .clone()
        .unwrap_or_else(|| format!("{}@{}", author.handle, author.lk_domain));
    let default_message = match kind {
        MergeKind::Squash => format!(
            "{} (#{})\n\n{}",
            pr.title,
            pr.number,
            pr.body.clone().unwrap_or_default()
        ),
        _ => format!(
            "Merge pull request #{} from {}\n\n{}",
            pr.number,
            pr.head_ref.trim_start_matches("refs/heads/"),
            pr.title
        ),
    };
    let msg = CommitMsg {
        message: req
            .message
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(default_message),
        author_name: match kind {
            MergeKind::Squash => author_name,
            _ => p
                .identity
                .display_name
                .clone()
                .unwrap_or_else(|| p.identity.handle.clone()),
        },
        author_email: match kind {
            MergeKind::Squash => author_email,
            _ => p
                .identity
                .email
                .clone()
                .filter(|_| p.identity.email_verified)
                .unwrap_or_else(|| {
                    format!(
                        "noreply+{}@{}",
                        p.identity.lk_user_id, app.config.instance_domain
                    )
                }),
        },
        committer_name: "vcsstar".into(),
        committer_email: format!("vcsstar@{}", app.config.instance_domain),
    };
    let mut trailers = vec![
        ("Vcsstar-Acting-Identity".to_string(), p.key()),
        (
            "Vcsstar-Acting-Handle".to_string(),
            format!("{}@{}", p.identity.handle, p.identity.lk_domain),
        ),
        (
            "Vcsstar-Pull-Request".to_string(),
            format!("#{}", pr.number),
        ),
    ];
    if let Some(via) = &p.acting_via {
        if p.kind == crate::principal::CredentialKind::ActAs {
            trailers.push(("Vcsstar-Acting-Via".to_string(), via.clone()));
        }
    }
    let signer = instance_keys::signer(app, &mut conn)?;
    let outcome = app.store.backend().merge(
        &dir,
        kind,
        &base_now,
        &RevId(pr.head_sha.clone()),
        &msg,
        signer.as_ref().map(|s| &s.signer),
        &trailers,
    )?;
    let merged = match outcome {
        MergeOutcome::Clean(id) => id,
        MergeOutcome::Conflicts(paths) => {
            diesel::update(pull_requests::table.find(pr.id))
                .set(pull_requests::mergeable.eq("conflicts"))
                .execute(&mut conn)?;
            return Err(Error::new(
                Code::Conflict,
                format!("merge conflicts in: {}", paths.join(", ")),
            ));
        }
    };
    let commits = crate::services::vcs_ops::summaries(app, &dir, Some(&base_now), &merged)?;
    let updated = conn.transaction::<PrRow, Error, _>(|conn| {
        vcs::capture(
            app,
            conn,
            repo.id,
            &dir,
            &[base_now.clone(), RevId(pr.head_sha.clone())],
            std::slice::from_ref(&merged),
        )?;
        let update = RefUpdate {
            name: pr.base_ref.clone(),
            old: Some(base_now.clone()),
            new: Some(merged.clone()),
        };
        push::apply_ref_change(conn, &org, &repo, Some(p), &update, false, commits)?;
        let u: PrRow = diesel::update(pull_requests::table.find(pr.id))
            .set((
                pull_requests::state.eq("merged"),
                pull_requests::merge_kind.eq(kind.as_str()),
                pull_requests::merged_commit_sha.eq(&merged.0),
                pull_requests::merged_by.eq(p.identity.id),
                pull_requests::mergeable.eq("clean"),
                pull_requests::closed_at.eq(Utc::now()),
                pull_requests::updated_at.eq(Utc::now()),
            ))
            .returning(PrRow::as_returning())
            .get_result(conn)?;
        drop_internal_refs(conn, repo.id, pr.number)?;
        outbox::emit(conn, pr_event(&org, &repo, Some(p), "pr_merged", &u))?;
        Ok(u)
    })?;
    Ok(MergeResult {
        pull_request: to_wire(&mut conn, &org, &repo, &updated)?,
        merged_commit: merged.0,
    })
}

/// Called after a push transaction: move head snapshots of open PRs whose
/// head ref changed, and invalidate mergeability for PRs on a moved base.
pub fn on_push(
    app: &App,
    conn: &mut DbConn,
    org: &OrgRow,
    repo: &RepoRow,
    actor: Option<&Principal>,
    updates: &[RefUpdate],
) -> Result<(), Error> {
    for u in updates {
        if !u.name.starts_with("refs/heads/") {
            continue;
        }
        // Heads: PRs whose head is this ref (same repo, or a fork pointing at base repos).
        let heads: Vec<(PrRow, RepoRow)> = pull_requests::table
            .inner_join(repos::table.on(repos::id.eq(pull_requests::repo_id)))
            .filter(pull_requests::state.eq("open"))
            .filter(pull_requests::head_ref.eq(&u.name))
            .filter(pull_requests::head_remote.is_null())
            .filter(
                pull_requests::head_repo_id
                    .eq(repo.id)
                    .or(pull_requests::head_repo_id
                        .is_null()
                        .and(pull_requests::repo_id.eq(repo.id))),
            )
            .select((PrRow::as_select(), RepoRow::as_select()))
            .load(conn)?;
        for (pr, base_repo) in heads {
            let base_org: OrgRow = vcsstar_db::schema::orgs::table
                .find(base_repo.org_id)
                .select(OrgRow::as_select())
                .first(conn)?;
            match &u.new {
                None => {
                    // Head branch deleted: the PR keeps its last snapshot.
                    continue;
                }
                Some(new) => {
                    // Inside a push the pushed repository's directory is
                    // already hydrated, and a materialize here would race
                    // git's own ref update (it deletes refs that are gone
                    // from Postgres before receive-pack writes them).
                    let base_dir = if base_repo.id == repo.id {
                        app.store.repo_dir(repo.id)
                    } else {
                        vcs::hydrate(app, conn, &base_repo)?
                    };
                    snapshot_head(app, conn, &base_repo, &base_dir, repo, new)?;
                    let cur = refs::get(conn, base_repo.id, &head_ref_name(pr.number))?;
                    refs::apply(
                        conn,
                        base_repo.id,
                        &[RefUpdate {
                            name: head_ref_name(pr.number),
                            old: cur,
                            new: Some(new.clone()),
                        }],
                        None,
                    )?;
                    let moved: PrRow = diesel::update(pull_requests::table.find(pr.id))
                        .set((
                            pull_requests::head_sha.eq(&new.0),
                            pull_requests::mergeable.eq("unknown"),
                            pull_requests::updated_at.eq(Utc::now()),
                        ))
                        .returning(PrRow::as_returning())
                        .get_result(conn)?;
                    outbox::emit(
                        conn,
                        pr_event(&base_org, &base_repo, actor, "pr_head_moved", &moved),
                    )?;
                }
            }
        }
        // Bases: PRs targeting this ref in this repo.
        match &u.new {
            Some(new) => {
                diesel::update(
                    pull_requests::table
                        .filter(pull_requests::repo_id.eq(repo.id))
                        .filter(pull_requests::state.eq("open"))
                        .filter(pull_requests::base_ref.eq(&u.name)),
                )
                .set((
                    pull_requests::base_sha.eq(&new.0),
                    pull_requests::mergeable.eq("unknown"),
                ))
                .execute(conn)?;
            }
            None => retarget_on_base_delete(app, conn, org, repo, actor, &u.name)?,
        }
    }
    Ok(())
}

/// A deleted base branch (the usual end of a stacked branch once it is
/// merged) moves the open PRs that targeted it onto the default branch,
/// the way GitHub does. Each one gets a fresh trial merge and a
/// `pr_retargeted` event.
fn retarget_on_base_delete(
    app: &App,
    conn: &mut DbConn,
    org: &OrgRow,
    repo: &RepoRow,
    actor: Option<&Principal>,
    deleted: &str,
) -> Result<(), Error> {
    let default_ref = format!("refs/heads/{}", repo.default_branch);
    if deleted == default_ref {
        return Ok(());
    }
    let open: Vec<PrRow> = pull_requests::table
        .filter(pull_requests::repo_id.eq(repo.id))
        .filter(pull_requests::state.eq("open"))
        .filter(pull_requests::base_ref.eq(deleted))
        .select(PrRow::as_select())
        .load(conn)?;
    if open.is_empty() {
        return Ok(());
    }
    let Some(base_sha) = refs::get(conn, repo.id, &default_ref)? else {
        return Ok(());
    };
    // The caller (push hook, drift import, PR import) hydrated the
    // directory already; see the note in `on_push`.
    let dir = app.store.repo_dir(repo.id);
    for pr in open {
        let u: PrRow = diesel::update(pull_requests::table.find(pr.id))
            .set((
                pull_requests::base_ref.eq(&default_ref),
                pull_requests::base_sha.eq(&base_sha.0),
                pull_requests::mergeable.eq("unknown"),
                pull_requests::updated_at.eq(Utc::now()),
            ))
            .returning(PrRow::as_returning())
            .get_result(conn)?;
        let u = trial_merge(app, conn, repo, &dir, &u)?;
        outbox::emit(conn, pr_event(org, repo, actor, "pr_retargeted", &u))?;
    }
    Ok(())
}
