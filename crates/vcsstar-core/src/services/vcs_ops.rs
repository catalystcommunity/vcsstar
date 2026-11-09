//! Repository content: refs, history, trees, blobs, diffs, forks, and
//! compare-and-swap ref updates through the API. Every read hydrates the
//! node cache from Postgres first; every write goes through the same
//! transaction shape as a push.

use diesel::prelude::*;
use vcsstar_db::models::{NewRepo, Org as OrgRow, Repo as RepoRow};
use vcsstar_db::schema::repos;
use vcsstar_store::hook::ZERO_ID;
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_types::{
    BlobRequest, BlobResponse, CaptureReport, Commit, CommitPage, CommitSummary, DiffRequest,
    DiffResponse, EventPayload, FileDiff, ForkRequest, GetCommitRequest, GetRepoRequest,
    ListRefsRequest, ListRefsResponse, LogRequest, RefInfo, Repo, SettingsPayload, TreeEntry,
    TreeRequest, TreeResponse, UpdateRefRequest,
};
use vcsstar_vcs::{Env, RevId};

use crate::authz::{self, OrgRole, RepoRole};
use crate::outbox::{self, Emit};
use crate::principal::ReqCtx;
use crate::services::keys::AllowedSigners;
use crate::services::{check_name, repos as repo_svc};
use crate::{push, vcs, App, Code, Error};

const MAX_BLOB_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PATCH_BYTES: usize = 512 * 1024;

pub fn list_refs(app: &App, ctx: &ReqCtx, req: ListRefsRequest) -> Result<ListRefsResponse, Error> {
    let mut conn = app.conn()?;
    let (_org, repo, _role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    let rows = refs::load(&mut conn, repo.id)?;
    let prefix = req.prefix.unwrap_or_default();
    let refs = rows
        .into_iter()
        .filter(|(n, _)| n.starts_with(&prefix))
        .filter(|(n, _)| !n.starts_with("refs/vcsstar/"))
        .map(|(name, target)| RefInfo {
            name,
            target: target.0,
            peeled: None,
        })
        .collect();
    Ok(ListRefsResponse {
        refs,
        refs_version: repo.refs_version.max(0) as u64,
    })
}

fn open(
    app: &App,
    ctx: &ReqCtx,
    org: &str,
    name: &str,
) -> Result<
    (
        vcsstar_db::DbConn,
        OrgRow,
        RepoRow,
        RepoRole,
        std::path::PathBuf,
    ),
    Error,
> {
    let mut conn = app.conn()?;
    let (org, repo, role) = authz::load_repo_visible(app, &mut conn, ctx, org, name)?;
    let dir = vcs::hydrate(app, &mut conn, &repo)?;
    Ok((conn, org, repo, role, dir))
}

pub fn log(app: &App, ctx: &ReqCtx, req: LogRequest) -> Result<CommitPage, Error> {
    let (mut conn, _org, _repo, _role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    let backend = app.store.backend();
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(30)
        .clamp(1, 200) as usize;
    let start = match req.page.as_ref().and_then(|p| p.cursor.as_deref()) {
        Some(c) => backend.resolve(&dir, c, &Env::new())?,
        None => backend.resolve(&dir, &req.rev, &Env::new())?,
    };
    let mut revs = backend.log(&dir, &start, req.path.as_deref(), limit + 1)?;
    let next_cursor = if revs.len() > limit {
        revs.truncate(limit);
        // The cursor is the next commit to start from; with a path filter the
        // next page re-runs the filtered walk from that commit.
        let last = &revs[limit - 1];
        match req.path {
            Some(_) => backend
                .log(&dir, &last.id, req.path.as_deref(), 2)?
                .get(1)
                .map(|r| r.id.0.clone()),
            None => last.parents.first().map(|p| p.0.clone()),
        }
    } else {
        None
    };
    let signers = AllowedSigners::build(&mut conn)?;
    let mut commits = Vec::with_capacity(revs.len());
    for r in &revs {
        let full = if r.signature.is_none() {
            // `log` does not carry the signature block; fetch it only when
            // the commit may be signed (cheap check on the raw object).
            backend.revision(&dir, &r.id, &Env::new())?
        } else {
            r.clone()
        };
        let sig = signers.classify(&mut conn, backend, &dir, &full, &Env::new())?;
        commits.push(vcs::commit_to_wire(&full, sig));
    }
    Ok(CommitPage {
        commits,
        next_cursor,
    })
}

pub fn get_commit(app: &App, ctx: &ReqCtx, req: GetCommitRequest) -> Result<Commit, Error> {
    let (mut conn, _org, _repo, _role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    let backend = app.store.backend();
    let id = backend.resolve(&dir, &req.rev, &Env::new())?;
    let rev = backend.revision(&dir, &id, &Env::new())?;
    let signers = AllowedSigners::build(&mut conn)?;
    let sig = signers.classify(&mut conn, backend, &dir, &rev, &Env::new())?;
    Ok(vcs::commit_to_wire(&rev, sig))
}

pub fn tree(app: &App, ctx: &ReqCtx, req: TreeRequest) -> Result<TreeResponse, Error> {
    let (_conn, _org, _repo, _role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    let backend = app.store.backend();
    let id = backend.resolve(&dir, &req.rev, &Env::new())?;
    let entries = backend.tree_at(&dir, &id, req.path.as_deref().unwrap_or(""))?;
    Ok(TreeResponse {
        entries: entries
            .into_iter()
            .map(|e| TreeEntry {
                name: e.name,
                kind: e.kind,
                id: e.id,
                mode: e.mode as u64,
                size: e.size,
            })
            .collect(),
        rev_id: id.0,
    })
}

pub fn blob(app: &App, ctx: &ReqCtx, req: BlobRequest) -> Result<BlobResponse, Error> {
    let (_conn, _org, _repo, _role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    let backend = app.store.backend();
    let id = backend.resolve(&dir, &req.rev, &Env::new())?;
    let (oid, content) = backend.blob(&dir, &id, &req.path)?;
    let size = content.len() as u64;
    let offset = req.offset.unwrap_or(0).min(size);
    let limit = req.limit.unwrap_or(MAX_BLOB_BYTES).min(MAX_BLOB_BYTES);
    let end = (offset + limit).min(size);
    let slice = content[offset as usize..end as usize].to_vec();
    let binary = content.iter().take(8192).any(|b| *b == 0);
    Ok(BlobResponse {
        id: oid,
        size,
        offset,
        truncated: end < size,
        binary,
        content: slice,
    })
}

pub fn diff(app: &App, ctx: &ReqCtx, req: DiffRequest) -> Result<DiffResponse, Error> {
    let (_conn, _org, _repo, _role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    let backend = app.store.backend();
    let base = backend.resolve(&dir, &req.base, &Env::new())?;
    let head = backend.resolve(&dir, &req.head, &Env::new())?;
    let stat = backend.diff_stat(&dir, &base, &head, req.path.as_deref())?;
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(50)
        .clamp(1, 500) as usize;
    let start: usize = req
        .page
        .as_ref()
        .and_then(|p| p.cursor.as_deref())
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let mut files = Vec::new();
    let mut budget = MAX_PATCH_BYTES;
    for f in stat.iter().skip(start).take(limit) {
        let path = f
            .new_path
            .clone()
            .or_else(|| f.old_path.clone())
            .unwrap_or_default();
        let patch = if f.status == "binary" || budget == 0 {
            None
        } else {
            let p = backend.diff_patch(&dir, &base, &head, &path)?;
            if p.len() <= budget {
                budget -= p.len();
                Some(p)
            } else {
                budget = 0;
                None
            }
        };
        files.push(FileDiff {
            status: f.status.clone(),
            old_path: f.old_path.clone(),
            new_path: f.new_path.clone(),
            additions: f.additions,
            deletions: f.deletions,
            patch,
        });
    }
    let next_cursor = (start + limit < stat.len()).then(|| (start + limit).to_string());
    Ok(DiffResponse { files, next_cursor })
}

/// `Repos/capture-history`: backfill the segments with history the object
/// store never received (written before the store existed, or after a failed
/// upload). Repository admins only.
pub fn capture_history(
    app: &App,
    ctx: &ReqCtx,
    req: GetRepoRequest,
) -> Result<CaptureReport, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    let (mut conn, org, repo, role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    if !app.store.has_segment_store() {
        return Err(Error::new(
            crate::errors::Code::Unavailable,
            "this instance has no object store",
        ));
    }
    let result = vcs::capture_missing(app, &mut conn, &repo, &dir)?;
    Ok(CaptureReport {
        repo: vcs::repo_ref(&org, &repo),
        uploaded: result.is_some(),
        objects: result.as_ref().map(|r| r.2 as u64).unwrap_or(0),
        bytes: result.as_ref().map(|r| r.1 as u64).unwrap_or(0),
        segment: result.map(|r| r.0),
    })
}

pub fn update_ref(app: &App, ctx: &ReqCtx, req: UpdateRefRequest) -> Result<RefInfo, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:write")?;
    vcsstar_vcs::check_ref_name(&req.name)?;
    if req.name.starts_with("refs/vcsstar/") {
        return Err(Error::forbidden("internal refs are not writable"));
    }
    let (mut conn, org, repo, role, dir) = open(app, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Write)?;
    if repo.archived {
        return Err(Error::forbidden("repository is archived"));
    }
    let backend = app.store.backend();
    let old = req.expected.as_deref().map(|s| RevId(s.to_string()));
    let new = match req.target.as_deref() {
        Some(t) => Some(backend.resolve(&dir, t, &Env::new())?),
        None => None,
    };
    if new.is_none() && req.name == format!("refs/heads/{}", repo.default_branch) {
        return Err(Error::forbidden("cannot delete the default branch"));
    }
    let forced = match (&old, &new) {
        (Some(o), Some(n)) => !backend
            .is_ancestor(&dir, o, n, &Env::new())
            .unwrap_or(false),
        _ => false,
    };
    // Protections apply as for a push.
    let protections = repo_svc::protections_for(&mut conn, repo.id)?;
    for pr in protections
        .iter()
        .filter(|pr| vcsstar_vcs::ref_pattern_matches(&pr.ref_pattern, &req.name))
    {
        if new.is_none() && !pr.allow_delete {
            return Err(Error::forbidden("ref is protected against deletion"));
        }
        if forced && !pr.allow_force_push {
            return Err(Error::forbidden("ref is protected against force updates"));
        }
    }
    let commits = match (&old, &new) {
        (_, Some(n)) => backend
            .rev_list(&dir, old.as_ref().filter(|_| !forced), n, 20, &Env::new())?
            .into_iter()
            .rev()
            .map(|c| CommitSummary {
                id: c.id.0,
                author_name: c.author_name,
                author_email: c.author_email,
                message_head: c.message.lines().next().unwrap_or("").to_string(),
                signature: "missing".into(),
            })
            .collect(),
        _ => vec![],
    };
    let update = RefUpdate {
        name: req.name.clone(),
        old,
        new: new.clone(),
    };
    conn.transaction::<(), Error, _>(|conn| {
        push::apply_ref_change(conn, &org, &repo, Some(p), &update, forced, commits)
    })?;
    Ok(RefInfo {
        name: req.name,
        target: new.map(|r| r.0).unwrap_or_else(|| ZERO_ID.to_string()),
        peeled: None,
    })
}

pub fn fork(app: &App, ctx: &ReqCtx, req: ForkRequest) -> Result<Repo, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    let name = req.name.clone().unwrap_or_else(|| req.source.name.clone());
    check_name("repository", &name)?;
    let mut conn = app.conn()?;
    let (src_org, src, _role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.source.org, &req.source.name)?;
    let dst_org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &dst_org, p, OrgRole::Admin)?;
    // Hydrate the source so the fork copies current objects.
    vcs::hydrate(app, &mut conn, &src)?;
    let dst_id = vcsstar_db::new_id();
    let visibility = if src.visibility == "public" {
        "public"
    } else {
        "org"
    };
    let repo = conn.transaction::<RepoRow, Error, _>(|conn| {
        let row = NewRepo {
            id: dst_id,
            org_id: dst_org.id,
            name: &name,
            visibility,
            vcs_type: &src.vcs_type,
            default_branch: &src.default_branch,
            description: src.description.as_deref(),
        };
        let repo: RepoRow = diesel::insert_into(repos::table)
            .values(&row)
            .returning(RepoRow::as_returning())
            .get_result(conn)?;
        diesel::update(repos::table.find(repo.id))
            .set(repos::fork_of_repo_id.eq(src.id))
            .execute(conn)?;
        refs::copy_all(conn, src.id, dst_id)?;
        vcsstar_store::segments::copy_all(conn, src.id, dst_id)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_repo(&dst_org.name, &repo.name),
                kind: "repo_created",
                repo_id: Some(repo.id),
                org_id: Some(dst_org.id),
                actor_id: Some(p.identity.id),
                actor: Some(p.to_wire()),
                repo: Some(vcs::repo_ref(&dst_org, &repo)),
                org: Some(dst_org.name.clone()),
                subject: None,
                payload: EventPayload::Variant5(SettingsPayload {
                    changed: vec![
                        "created".into(),
                        format!("fork_of:{}/{}", src_org.name, src.name),
                    ],
                }),
            },
        )?;
        Ok(repo)
    })?;
    app.store.fork(src.id, dst_id)?;
    let mut wire = repo_svc::to_wire(app, &dst_org, &repo);
    wire.fork_of = Some(vcs::repo_ref(&src_org, &src));
    Ok(wire)
}

/// The commit summaries for a push event, used by the merge engine too.
pub fn summaries(
    app: &App,
    dir: &std::path::Path,
    old: Option<&RevId>,
    new: &RevId,
) -> Result<Vec<CommitSummary>, Error> {
    let list = app
        .store
        .backend()
        .rev_list(dir, old, new, 20, &Env::new())?;
    Ok(list
        .into_iter()
        .rev()
        .map(|c| CommitSummary {
            id: c.id.0,
            author_name: c.author_name,
            author_email: c.author_email,
            message_head: c.message.lines().next().unwrap_or("").to_string(),
            signature: "missing".into(),
        })
        .collect())
}

pub fn code_for(e: &Error) -> Code {
    e.code
}
