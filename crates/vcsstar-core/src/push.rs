//! The push transaction. git runs `pre-receive` with the incoming pack in
//! quarantine and no ref moved; the hook forwards the ref updates to this
//! process over a Unix socket, and this module decides, in one Postgres
//! transaction: compare-and-swap every ref, write the outbox events. Any
//! failure rejects the whole push and nothing durable changed.

use std::collections::HashMap;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diesel::prelude::*;
use uuid::Uuid;
use vcsstar_db::models::{Org, Repo};
use vcsstar_db::schema::branch_protections;
use vcsstar_store::hook::{self, HookRefUpdate, HookRequest, HookResponse, ZERO_ID};
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_types::{CommitSummary, EventPayload, PushPayload, RefPayload};
use vcsstar_vcs::{ref_pattern_matches, Env, RevId, RevisionInfo, SigStatus};

use crate::authz::RepoRole;
use crate::outbox::{self, Emit};
use crate::principal::Principal;
use crate::services::keys;
use crate::{vcs, App, Error};

/// Everything the hook handler needs about a push in flight.
pub struct PushCtx {
    pub id: Uuid,
    pub org: Org,
    pub repo: Repo,
    pub dir: PathBuf,
    pub principal: Principal,
    pub role: RepoRole,
    pub request_id: String,
}

#[derive(Default)]
pub struct PushRegistry {
    inner: Mutex<HashMap<Uuid, Arc<PushCtx>>>,
}

/// Removes the push from the registry when the serving request ends.
pub struct PushGuard {
    registry: Arc<PushRegistry>,
    pub ctx: Arc<PushCtx>,
}

impl Drop for PushGuard {
    fn drop(&mut self) {
        self.registry.inner.lock().unwrap().remove(&self.ctx.id);
    }
}

impl PushRegistry {
    pub fn begin(self: &Arc<Self>, ctx: PushCtx) -> PushGuard {
        let ctx = Arc::new(ctx);
        self.inner.lock().unwrap().insert(ctx.id, Arc::clone(&ctx));
        PushGuard {
            registry: Arc::clone(self),
            ctx,
        }
    }

    pub fn get(&self, id: Uuid) -> Option<Arc<PushCtx>> {
        self.inner.lock().unwrap().get(&id).cloned()
    }
}

/// Environment for the `git receive-pack` process of one push.
pub fn receive_env(app: &App, push_id: Uuid) -> Env {
    vec![
        (hook::ENV_PUSH_ID.into(), push_id.to_string()),
        (
            hook::ENV_SOCKET.into(),
            app.config.hook_socket.display().to_string(),
        ),
    ]
}

/// Listen for hook callbacks until `stop` is set.
pub fn serve_hook_socket(app: App, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    let path: &Path = &app.config.hook_socket;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    listener.set_nonblocking(true)?;
    tracing::info!(path = %path.display(), "hook socket listening");
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let app = app.clone();
                std::thread::Builder::new()
                    .name("hook".into())
                    .spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let resp = match hook::read_request(&mut stream) {
                            Ok(req) => handle(&app, req),
                            Err(e) => HookResponse {
                                ok: false,
                                messages: vec![format!("vcsstar: bad hook request: {e}")],
                            },
                        };
                        if let Err(e) = hook::write_response(&mut stream, &resp) {
                            tracing::warn!(error = %e, "hook response write failed");
                        }
                    })?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    let _ = std::fs::remove_file(path);
    Ok(())
}

fn reject(msg: impl Into<String>) -> HookResponse {
    HookResponse {
        ok: false,
        messages: vec![format!("vcsstar: {}", msg.into())],
    }
}

pub fn handle(app: &App, req: HookRequest) -> HookResponse {
    if req.hook != "pre-receive" {
        return HookResponse {
            ok: true,
            messages: vec![],
        };
    }
    let Some(ctx) = Uuid::parse_str(&req.push_id)
        .ok()
        .and_then(|id| app.pushes.get(id))
    else {
        return reject("push context unknown; retry the push");
    };
    match pre_receive(app, &ctx, &req) {
        Ok(messages) => HookResponse { ok: true, messages },
        Err(e) => {
            tracing::info!(
                repo = %format!("{}/{}", ctx.org.name, ctx.repo.name),
                actor = %ctx.principal.key(),
                error = %e.message,
                "push rejected"
            );
            reject(e.message)
        }
    }
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = branch_protections)]
struct Protection {
    ref_pattern: String,
    required_approvals: i32,
    required_signature_level: String,
    required_checks: Vec<Option<String>>,
    allow_force_push: bool,
    allow_delete: bool,
}

struct Planned {
    update: RefUpdate,
    forced: bool,
    commits: Vec<RevisionInfo>,
    kind: &'static str,
}

/// Policy, then one transaction. Returns messages for the pusher.
fn pre_receive(app: &App, ctx: &PushCtx, req: &HookRequest) -> Result<Vec<String>, Error> {
    if req.updates.is_empty() {
        return Ok(vec![]);
    }
    if ctx.role < RepoRole::Write {
        return Err(Error::forbidden("write access required"));
    }
    if ctx.repo.archived {
        return Err(Error::forbidden("repository is archived"));
    }
    let backend = app.store.backend();
    let env: Env = req.env.clone();
    let mut conn = app.conn()?;
    let protections: Vec<Protection> = branch_protections::table
        .filter(branch_protections::repo_id.eq(ctx.repo.id))
        .select(Protection::as_select())
        .load(&mut conn)?;
    let default_ref = format!("refs/heads/{}", ctx.repo.default_branch);

    let mut planned = Vec::with_capacity(req.updates.len());
    let mut messages = Vec::new();
    let mut needs_signature_check = false;
    for HookRefUpdate { old, new, name } in &req.updates {
        vcsstar_vcs::check_ref_name(name)?;
        if name.starts_with("refs/vcsstar/") || name.starts_with("refs/pull/") {
            return Err(Error::forbidden(format!("{name} is an internal ref")));
        }
        let old = (old != ZERO_ID).then(|| RevId(old.clone()));
        let new = (new != ZERO_ID).then(|| RevId(new.clone()));
        let matching: Vec<&Protection> = protections
            .iter()
            .filter(|p| ref_pattern_matches(&p.ref_pattern, name))
            .collect();
        let is_tag = name.starts_with("refs/tags/");
        let is_head = name.starts_with("refs/heads/");
        let mut forced = false;
        let mut commits = Vec::new();
        let kind;
        match (&old, &new) {
            (_, None) => {
                if *name == default_ref {
                    return Err(Error::forbidden("cannot delete the default branch"));
                }
                if matching.iter().any(|p| !p.allow_delete) {
                    return Err(Error::forbidden(format!(
                        "{name} is protected against deletion"
                    )));
                }
                if ctx.role < RepoRole::Maintain && !matching.is_empty() {
                    return Err(Error::forbidden(format!(
                        "deleting protected ref {name} needs maintain"
                    )));
                }
                kind = if is_tag { "tag_delete" } else { "ref_delete" };
            }
            (None, Some(n)) => {
                if !backend.object_exists(&ctx.dir, n, &env) {
                    return Err(Error::invalid(format!(
                        "{name}: object {n} was not received"
                    )));
                }
                commits = backend.rev_list(&ctx.dir, None, n, 200, &env)?;
                kind = if is_tag { "tag_create" } else { "push" };
            }
            (Some(o), Some(n)) => {
                if !backend.object_exists(&ctx.dir, n, &env) {
                    return Err(Error::invalid(format!(
                        "{name}: object {n} was not received"
                    )));
                }
                let ff = backend.is_ancestor(&ctx.dir, o, n, &env).unwrap_or(false);
                forced = !ff;
                if is_tag {
                    // Moving a tag is always a force.
                    forced = true;
                }
                if forced && matching.iter().any(|p| !p.allow_force_push) {
                    return Err(Error::forbidden(format!(
                        "{name} is protected against force pushes (non-fast-forward)"
                    )));
                }
                commits = if ff {
                    backend.rev_list(&ctx.dir, Some(o), n, 200, &env)?
                } else {
                    backend.rev_list(&ctx.dir, None, n, 50, &env)?
                };
                kind = if is_tag { "tag_create" } else { "push" };
            }
        }
        if is_head && new.is_some() {
            // Review and status gates apply to PR merges. A ref that carries
            // them refuses direct pushes, as GitHub does.
            if let Some(p) = matching.iter().find(|p| {
                p.required_approvals > 0 || p.required_checks.iter().flatten().count() > 0
            }) {
                if ctx.role < RepoRole::Admin {
                    return Err(Error::forbidden(format!(
                        "{name} requires a pull request (protection {})",
                        p.ref_pattern
                    )));
                }
                messages.push(format!(
                    "vcsstar: admin override: direct push to protected {name}"
                ));
            }
        }
        if matching
            .iter()
            .any(|p| p.required_signature_level != "none")
            && new.is_some()
        {
            needs_signature_check = true;
        }
        planned.push(Planned {
            update: RefUpdate {
                name: name.clone(),
                old,
                new,
            },
            forced,
            commits,
            kind,
        });
    }

    // Signature checks over every new commit of protected refs.
    let allowed = if needs_signature_check || planned.iter().any(|p| !p.commits.is_empty()) {
        Some(keys::AllowedSigners::build(&mut conn)?)
    } else {
        None
    };
    if needs_signature_check {
        let allowed = allowed.as_ref().expect("built above");
        for p in &planned {
            let Some(new) = &p.update.new else { continue };
            let level = protections
                .iter()
                .filter(|pr| ref_pattern_matches(&pr.ref_pattern, &p.update.name))
                .map(|pr| pr.required_signature_level.as_str())
                .fold("none", |acc, l| match (acc, l) {
                    ("attested", _) | (_, "attested") => "attested",
                    ("any", _) | (_, "any") => "any",
                    _ => "none",
                });
            if level == "none" {
                continue;
            }
            let commits = backend.rev_list(&ctx.dir, p.update.old.as_ref(), new, 1000, &env)?;
            for c in &commits {
                let status = allowed.classify(&mut conn, backend, &ctx.dir, c, &env)?;
                let ok = match level {
                    "attested" => status.status == "verified",
                    _ => status.status == "verified" || status.status == "valid_unattested",
                };
                if !ok {
                    return Err(Error::forbidden(format!(
                        "{}: commit {} signature is {} (ref requires {})",
                        p.update.name, c.id, status.status, level
                    )));
                }
            }
        }
    }

    // Summaries for the event payloads (oldest first, capped).
    let mut summaries: Vec<Vec<CommitSummary>> = Vec::with_capacity(planned.len());
    for p in &planned {
        let mut list = Vec::new();
        for c in p.commits.iter().take(20) {
            let sig = match &allowed {
                Some(a) => a.classify(&mut conn, backend, &ctx.dir, c, &env)?.status,
                None => "missing".to_string(),
            };
            list.push(CommitSummary {
                id: c.id.0.clone(),
                author_name: c.author_name.clone(),
                author_email: c.author_email.clone(),
                message_head: c.message_head(),
                signature: sig,
            });
        }
        list.reverse();
        summaries.push(list);
    }

    // The incoming pack becomes a history segment before anything durable
    // changes; a failed transaction leaves an orphan the gc reclaims.
    let mut segment: Option<(String, i64, String)> = None;
    if app.store.has_segment_store() {
        let quarantine = env
            .iter()
            .find(|(k, _)| k == "GIT_QUARANTINE_PATH")
            .map(|(_, v)| std::path::PathBuf::from(v));
        if let Some(q) = quarantine {
            let packs = vcsstar_store::segments::packs_in(&q.join("pack"))?;
            if let Some(files) = packs.first() {
                segment = Some(app.store.upload_pack(ctx.repo.id, files)?);
            } else {
                tracing::warn!(repo = %ctx.repo.id, "push arrived without a quarantine pack; objects stay on this node until repack");
            }
        }
    }

    let updates: Vec<RefUpdate> = planned.iter().map(|p| p.update.clone()).collect();
    let actor = ctx.principal.to_wire();
    conn.transaction::<(), Error, _>(|conn| {
        if let Some((key, size, hash)) = &segment {
            vcsstar_store::segments::append(conn, ctx.repo.id, key, *size, hash, "pack")?;
        }
        refs::apply(conn, ctx.repo.id, &updates, Some(ctx.principal.identity.id)).map_err(|e| {
            match e {
                vcsstar_store::StoreError::Cas(m) => Error::new(
                    crate::Code::Conflict,
                    format!("{m}; fetch first and push again"),
                ),
                other => other.into(),
            }
        })?;
        for (p, commits) in planned.iter().zip(summaries) {
            let before = p
                .update
                .old
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| ZERO_ID.to_string());
            let after = p
                .update
                .new
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| ZERO_ID.to_string());
            let payload = match p.kind {
                "push" => EventPayload::Variant0(PushPayload {
                    r#ref: p.update.name.clone(),
                    before,
                    after,
                    forced: p.forced,
                    commits,
                }),
                _ => EventPayload::Variant1(RefPayload {
                    r#ref: p.update.name.clone(),
                    before: p.update.old.as_ref().map(|r| r.0.clone()),
                    after: p.update.new.as_ref().map(|r| r.0.clone()),
                }),
            };
            outbox::emit(
                conn,
                Emit {
                    lane: outbox::lane_for_repo(&ctx.org.name, &ctx.repo.name),
                    kind: p.kind,
                    repo_id: Some(ctx.repo.id),
                    org_id: Some(ctx.org.id),
                    actor_id: Some(ctx.principal.identity.id),
                    actor: Some(actor.clone()),
                    repo: Some(vcs::repo_ref(&ctx.org, &ctx.repo)),
                    org: Some(ctx.org.name.clone()),
                    subject: Some(p.update.name.clone()),
                    payload,
                },
            )?;
        }
        Ok(())
    })?;
    // PR snapshots follow the push; a failure here must not undo the push.
    if let Err(e) = crate::services::prs::on_push(app, &mut conn, &ctx.org, &ctx.repo, &updates) {
        tracing::warn!(error = %e.message, "pull request snapshot update failed after push");
        messages.push(format!(
            "vcsstar: note: pull request snapshots were not updated ({})",
            e.message
        ));
    }
    Ok(messages)
}

/// Apply a ref change that did not come through git (API `update-ref`,
/// merges). Same transaction shape as the hook; the caller has checked
/// authorization and object existence.
pub fn apply_ref_change(
    conn: &mut PgConnection,
    org: &Org,
    repo: &Repo,
    actor: Option<&Principal>,
    update: &RefUpdate,
    forced: bool,
    commits: Vec<CommitSummary>,
) -> Result<(), Error> {
    let is_tag = update.name.starts_with("refs/tags/");
    let kind = match (&update.old, &update.new) {
        (_, None) if is_tag => "tag_delete",
        (_, None) => "ref_delete",
        _ if is_tag => "tag_create",
        _ => "push",
    };
    refs::apply(
        conn,
        repo.id,
        std::slice::from_ref(update),
        actor.map(|p| p.identity.id),
    )?;
    let payload = match kind {
        "push" => EventPayload::Variant0(PushPayload {
            r#ref: update.name.clone(),
            before: update
                .old
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| ZERO_ID.to_string()),
            after: update
                .new
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| ZERO_ID.to_string()),
            forced,
            commits,
        }),
        _ => EventPayload::Variant1(RefPayload {
            r#ref: update.name.clone(),
            before: update.old.as_ref().map(|r| r.0.clone()),
            after: update.new.as_ref().map(|r| r.0.clone()),
        }),
    };
    outbox::emit(
        conn,
        Emit {
            lane: outbox::lane_for_repo(&org.name, &repo.name),
            kind,
            repo_id: Some(repo.id),
            org_id: Some(org.id),
            actor_id: actor.map(|p| p.identity.id),
            actor: actor.map(|p| p.to_wire()),
            repo: Some(vcs::repo_ref(org, repo)),
            org: Some(org.name.clone()),
            subject: Some(update.name.clone()),
            payload,
        },
    )?;
    Ok(())
}

/// Signature status of one revision for display and event payloads.
pub fn signature_status(sig: &SigStatus) -> &'static str {
    match sig {
        SigStatus::Good { .. } => "valid_unattested",
        SigStatus::Bad { .. } => "unverified",
        SigStatus::Missing => "missing",
    }
}
