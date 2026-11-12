//! Mirror service: status, blocked list and replay, credentials, pause and
//! resume, configure, reconcile, import. The engine is `crate::mirror`.

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_db::models::{NewRepo, Org as OrgRow, Repo as RepoRow};
use vcsstar_db::schema::{credentials, mirror_actions, mirror_state, repos};
use vcsstar_store::refs;
use vcsstar_types::{
    BlockedAction, ConfigureMirrorRequest, Credential, CredentialRef, EventPayload, GetRepoRequest,
    GithubHealth, GithubInstallation, GithubInstallationList, GithubRepoInfo, GithubRepoPage,
    ImportAllRequest, ImportAllResult, ImportRequest, ListBlockedResponse, ListCredentialsRequest,
    ListCredentialsResponse, ListGithubReposRequest, MirrorStatus, Ok as WireOk, ReconcileReport,
    ReplayRequest, ReplayResult, Repo, SetCredentialRequest, SetMirrorScheduleRequest,
    SettingsPayload,
};

use crate::authz::{self, OrgRole, RepoRole};
use crate::mirror::{self, cred, ActionRow, CredentialRow, StateRow};
use crate::mirror_import;
use crate::outbox::{self, Emit};
use crate::principal::ReqCtx;
use crate::services::{check_name, repos as repo_svc};
use crate::{now_ms, to_ms, vcs, App, Code, Error};

fn status_wire(
    conn: &mut PgConnection,
    org: &OrgRow,
    repo: &RepoRow,
    s: &StateRow,
) -> Result<MirrorStatus, Error> {
    let pending: i64 = mirror_actions::table
        .filter(mirror_actions::repo_id.eq(repo.id))
        .filter(mirror_actions::state.eq("pending"))
        .count()
        .get_result(conn)?;
    let blocked: i64 = mirror_actions::table
        .filter(mirror_actions::repo_id.eq(repo.id))
        .filter(mirror_actions::state.eq("blocked"))
        .count()
        .get_result(conn)?;
    Ok(MirrorStatus {
        repo: vcs::repo_ref(org, repo),
        github_repo: Some(s.github_repo.clone()),
        mode: s.mode.clone(),
        paused: s.paused || !repo.mirror_enabled,
        cursor_seq: s.cursor_seq.max(0) as u64,
        pending: pending.max(0) as u64,
        blocked: blocked.max(0) as u64,
        last_success_at: s.last_success_at.map(to_ms),
        last_error: s.last_error.clone(),
        last_full_reconcile_at: s.last_full_reconcile_at.map(to_ms),
        last_pull_sync_at: s.last_pull_sync_at.map(to_ms),
        backfill_state: Some(
            if s.backfill_done_at.is_some() {
                "done"
            } else if s.backfill_started_at.is_some() {
                "running"
            } else {
                "pending"
            }
            .into(),
        ),
        backfill_pulls: Some(s.backfill_pulls.max(0) as u64),
        backfill_started_at: s.backfill_started_at.map(to_ms),
        backfill_done_at: s.backfill_done_at.map(to_ms),
        import_state: mirror_import::state(repo, s).map(str::to_string),
        pull_sync_interval_s: mirror::pull_sync_interval(s).num_seconds() as u64,
        ref_reconcile_interval_s: mirror::ref_reconcile_interval(s).num_seconds() as u64,
    })
}

fn open_admin(
    app: &App,
    ctx: &ReqCtx,
    org: &str,
    name: &str,
    min: RepoRole,
) -> Result<(vcsstar_db::DbConn, OrgRow, RepoRow), Error> {
    let p = ctx.require()?;
    p.require_scope("mirror:manage")?;
    let mut conn = app.conn()?;
    let (org, repo, role) = authz::load_repo_visible(app, &mut conn, ctx, org, name)?;
    authz::require_repo_role(role, min)?;
    Ok((conn, org, repo))
}

pub fn status(app: &App, ctx: &ReqCtx, req: GetRepoRequest) -> Result<MirrorStatus, Error> {
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Read)?;
    match mirror::state_for(&mut conn, repo.id)? {
        Some(s) => status_wire(&mut conn, &org, &repo, &s),
        None => Ok(MirrorStatus {
            repo: vcs::repo_ref(&org, &repo),
            github_repo: None,
            mode: "authoritative_vcsstar".into(),
            paused: true,
            cursor_seq: 0,
            pending: 0,
            blocked: 0,
            last_success_at: None,
            last_error: Some("mirror not configured".into()),
            last_full_reconcile_at: None,
            last_pull_sync_at: None,
            backfill_state: None,
            backfill_pulls: None,
            backfill_started_at: None,
            backfill_done_at: None,
            import_state: None,
            pull_sync_interval_s: mirror::DEFAULT_PASS_INTERVAL_S as u64,
            ref_reconcile_interval_s: mirror::DEFAULT_PASS_INTERVAL_S as u64,
        }),
    }
}

pub fn configure(
    app: &App,
    ctx: &ReqCtx,
    req: ConfigureMirrorRequest,
) -> Result<MirrorStatus, Error> {
    let (mut conn, org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Admin)?;
    let (owner, name) = req
        .github_repo
        .split_once('/')
        .ok_or_else(|| Error::invalid("github_repo must be owner/name"))?;
    if owner.is_empty() || name.is_empty() || req.github_repo.contains(char::is_whitespace) {
        return Err(Error::invalid("github_repo must be owner/name"));
    }
    let mode = req.mode.as_deref().unwrap_or("authoritative_vcsstar");
    if !mirror::MODES.contains(&mode) {
        return Err(Error::invalid(format!(
            "mode must be one of {:?}",
            mirror::MODES
        )));
    }
    let enabled = req.enabled.unwrap_or(true);
    let state = conn.transaction::<StateRow, Error, _>(|conn| {
        let s: StateRow = diesel::insert_into(mirror_state::table)
            .values((
                mirror_state::repo_id.eq(repo.id),
                mirror_state::github_repo.eq(&req.github_repo),
                mirror_state::mode.eq(mode),
            ))
            .on_conflict(mirror_state::repo_id)
            .do_update()
            .set((
                mirror_state::github_repo.eq(&req.github_repo),
                mirror_state::mode.eq(mode),
            ))
            .returning(StateRow::as_returning())
            .get_result(conn)?;
        diesel::update(repos::table.find(repo.id))
            .set(repos::mirror_enabled.eq(enabled))
            .execute(conn)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_repo(&org.name, &repo.name),
                kind: "repo_settings_changed",
                repo_id: Some(repo.id),
                org_id: Some(org.id),
                actor_id: ctx.actor_id(),
                actor: ctx.principal.as_ref().map(|p| p.to_wire()),
                repo: Some(vcs::repo_ref(&org, &repo)),
                org: Some(org.name.clone()),
                subject: None,
                payload: EventPayload::Variant5(SettingsPayload {
                    changed: vec!["mirror".into()],
                }),
            },
        )?;
        Ok(s)
    })?;
    let repo = repos::table
        .find(repo.id)
        .select(RepoRow::as_select())
        .first(&mut conn)?;
    status_wire(&mut conn, &org, &repo, &state)
}

pub fn pause(app: &App, ctx: &ReqCtx, req: GetRepoRequest) -> Result<WireOk, Error> {
    let (mut conn, _org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Admin)?;
    let n = diesel::update(mirror_state::table.find(repo.id))
        .set(mirror_state::paused.eq(true))
        .execute(&mut conn)?;
    if n == 0 {
        return Err(Error::new(
            Code::PreconditionFailed,
            "mirror not configured",
        ));
    }
    Ok(WireOk { ok: true })
}

pub fn resume(app: &App, ctx: &ReqCtx, req: GetRepoRequest) -> Result<WireOk, Error> {
    let (mut conn, org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Admin)?;
    let n = diesel::update(mirror_state::table.find(repo.id))
        .set(mirror_state::paused.eq(false))
        .execute(&mut conn)?;
    if n == 0 {
        return Err(Error::new(
            Code::PreconditionFailed,
            "mirror not configured",
        ));
    }
    // Wake every pending action that waited out the pause.
    let pending: Vec<(uuid::Uuid, i64)> = mirror_actions::table
        .filter(mirror_actions::repo_id.eq(repo.id))
        .filter(mirror_actions::state.eq("pending"))
        .select((mirror_actions::id, mirror_actions::event_seq))
        .load(&mut conn)?;
    for (id, _seq) in pending {
        if let Err(e) = mirror::submit_action(app, &org.name, &repo.name, id, now_ms()) {
            tracing::warn!(error = %e.message, "resume: task submit failed; reconcile will catch up");
        }
    }
    Ok(WireOk { ok: true })
}

pub fn list_blocked(
    app: &App,
    ctx: &ReqCtx,
    req: GetRepoRequest,
) -> Result<ListBlockedResponse, Error> {
    let mut conn = app.conn()?;
    let (_org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Write)?;
    let rows: Vec<ActionRow> = mirror_actions::table
        .filter(mirror_actions::repo_id.eq(repo.id))
        .filter(mirror_actions::state.eq("blocked"))
        .order(mirror_actions::blocked_at.asc())
        .select(ActionRow::as_select())
        .load(&mut conn)?;
    Ok(ListBlockedResponse {
        actions: rows
            .iter()
            .map(|a| BlockedAction {
                id: a.id.to_string(),
                event_seq: a.event_seq.max(0) as u64,
                action_kind: a.action_kind.clone(),
                object_key: a.object_key.clone(),
                error: a.blocked_reason.clone().unwrap_or_default(),
                blocked_at: a.blocked_at.map(to_ms).unwrap_or_else(now_ms),
                credential_purpose: a.credential_purpose.clone(),
            })
            .collect(),
    })
}

pub fn replay_blocked(app: &App, ctx: &ReqCtx, req: ReplayRequest) -> Result<ReplayResult, Error> {
    let (mut conn, org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Maintain)?;
    let mut q = mirror_actions::table
        .filter(mirror_actions::repo_id.eq(repo.id))
        .filter(mirror_actions::state.eq("blocked"))
        .into_boxed();
    if !req.action_ids.is_empty() {
        let ids: Vec<uuid::Uuid> = req
            .action_ids
            .iter()
            .map(|s| uuid::Uuid::parse_str(s).map_err(|_| Error::invalid("action id")))
            .collect::<Result<_, _>>()?;
        q = q.filter(mirror_actions::id.eq_any(ids));
    }
    let rows: Vec<ActionRow> = q
        .order(mirror_actions::event_seq.asc())
        .select(ActionRow::as_select())
        .load(&mut conn)?;
    let mut requeued = 0u64;
    for a in rows {
        // A newer pending action for the same key supersedes this one.
        let newer: Option<uuid::Uuid> = mirror_actions::table
            .filter(mirror_actions::repo_id.eq(repo.id))
            .filter(mirror_actions::object_key.eq(&a.object_key))
            .filter(mirror_actions::state.eq("pending"))
            .select(mirror_actions::id)
            .first(&mut conn)
            .optional()?;
        let (target_id, seq) = match newer {
            Some(id) => {
                diesel::update(mirror_actions::table.find(a.id))
                    .set((
                        mirror_actions::state.eq("skipped"),
                        mirror_actions::updated_at.eq(Utc::now()),
                    ))
                    .execute(&mut conn)?;
                (id, a.event_seq)
            }
            None => {
                diesel::update(mirror_actions::table.find(a.id))
                    .set((
                        mirror_actions::state.eq("pending"),
                        mirror_actions::blocked_reason.eq::<Option<String>>(None),
                        mirror_actions::blocked_at.eq::<Option<chrono::DateTime<Utc>>>(None),
                        mirror_actions::updated_at.eq(Utc::now()),
                    ))
                    .execute(&mut conn)?;
                (a.id, a.event_seq)
            }
        };
        let _ = seq;
        mirror::submit_action(app, &org.name, &repo.name, target_id, now_ms())?;
        requeued += 1;
    }
    Ok(ReplayResult { requeued })
}

fn cred_wire(conn: &mut PgConnection, c: &CredentialRow) -> Result<Credential, Error> {
    let org: String = vcsstar_db::schema::orgs::table
        .find(c.org_id)
        .select(vcsstar_db::schema::orgs::name)
        .first(conn)?;
    let repo: Option<String> = match c.repo_id {
        Some(r) => repos::table
            .find(r)
            .select(repos::name)
            .first(conn)
            .optional()?,
        None => None,
    };
    Ok(Credential {
        id: c.id.to_string(),
        org,
        repo,
        provider: c.provider.clone(),
        purpose: c.purpose.clone(),
        auth_method: c.auth_method.clone(),
        host: c.host.clone(),
        secret_ref: c.secret_ref.clone(),
        enabled: c.enabled,
    })
}

fn authorize_cred_target(
    app: &App,
    conn: &mut vcsstar_db::DbConn,
    ctx: &ReqCtx,
    org: &str,
    repo: Option<&str>,
) -> Result<(uuid::Uuid, Option<uuid::Uuid>), Error> {
    let p = ctx.require()?;
    p.require_scope("mirror:manage")?;
    let org_row = authz::load_org(conn, org)?;
    match repo {
        Some(r) => {
            let (_o, repo_row, role) = authz::load_repo_visible(app, conn, ctx, org, r)?;
            authz::require_repo_role(role, RepoRole::Admin)?;
            Ok((org_row.id, Some(repo_row.id)))
        }
        None => {
            authz::require_org_role(app, conn, &org_row, p, OrgRole::Admin)?;
            Ok((org_row.id, None))
        }
    }
}

pub fn set_credential(
    app: &App,
    ctx: &ReqCtx,
    req: SetCredentialRequest,
) -> Result<Credential, Error> {
    let mut conn = app.conn()?;
    let (org_id, repo_id) =
        authorize_cred_target(app, &mut conn, ctx, &req.org, req.repo.as_deref())?;
    if req.provider != "github" {
        return Err(Error::invalid("provider must be github"));
    }
    if !mirror::PURPOSES.contains(&req.purpose.as_str()) {
        return Err(Error::invalid(format!(
            "purpose must be one of {:?}",
            mirror::PURPOSES
        )));
    }
    if !["pat", "github_app"].contains(&req.auth_method.as_str()) {
        return Err(Error::invalid("auth_method must be pat or github_app"));
    }
    if req.auth_method == "github_app" && req.purpose == "webhook_inbound" {
        return Err(Error::invalid(
            "webhook_inbound is a shared secret; it has no github_app form",
        ));
    }
    let ciphertext = if req.secret_ref == "stored" {
        let secret = req
            .secret
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::invalid("secret is required when secret_ref is stored"))?;
        if req.auth_method == "github_app" {
            crate::github_app::AppCredential::parse(secret)?;
        }
        Some(cred::seal(app, secret.trim().as_bytes())?)
    } else if let Some(var) = req.secret_ref.strip_prefix("env:") {
        if var.is_empty() || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::invalid("env: ref must name an environment variable"));
        }
        // Check the value now when the variable is already set on this node.
        if req.auth_method == "github_app" {
            if let Ok(v) = std::env::var(var) {
                crate::github_app::AppCredential::parse(&v)?;
            }
        }
        None
    } else {
        return Err(Error::invalid("secret_ref must be `stored` or `env:<VAR>`"));
    };
    let row: CredentialRow = diesel::insert_into(credentials::table)
        .values((
            credentials::id.eq(vcsstar_db::new_id()),
            credentials::org_id.eq(org_id),
            credentials::repo_id.eq(repo_id),
            credentials::provider.eq(&req.provider),
            credentials::purpose.eq(&req.purpose),
            credentials::auth_method.eq(&req.auth_method),
            credentials::host.eq(&req.host),
            credentials::secret_ref.eq(&req.secret_ref),
            credentials::secret_ciphertext.eq(ciphertext),
            credentials::enabled.eq(req.enabled.unwrap_or(true)),
            credentials::created_by.eq(ctx.actor_id()),
        ))
        .returning(CredentialRow::as_returning())
        .get_result(&mut conn)?;
    cred_wire(&mut conn, &row)
}

pub fn list_credentials(
    app: &App,
    ctx: &ReqCtx,
    req: ListCredentialsRequest,
) -> Result<ListCredentialsResponse, Error> {
    let mut conn = app.conn()?;
    let (org_id, repo_id) = authorize_cred_target(
        app,
        &mut conn,
        ctx,
        &req.target.org,
        req.target.repo.as_deref(),
    )?;
    let mut q = credentials::table
        .filter(credentials::org_id.eq(org_id))
        .into_boxed();
    if let Some(r) = repo_id {
        q = q.filter(
            credentials::repo_id
                .eq(r)
                .or(credentials::repo_id.is_null()),
        );
    }
    let rows: Vec<CredentialRow> = q
        .order(credentials::created_at.asc())
        .select(CredentialRow::as_select())
        .load(&mut conn)?;
    let mut out = Vec::new();
    for r in &rows {
        out.push(cred_wire(&mut conn, r)?);
    }
    Ok(ListCredentialsResponse { credentials: out })
}

pub fn remove_credential(app: &App, ctx: &ReqCtx, req: CredentialRef) -> Result<WireOk, Error> {
    let mut conn = app.conn()?;
    let id =
        uuid::Uuid::parse_str(&req.credential_id).map_err(|_| Error::invalid("credential_id"))?;
    let row: CredentialRow = credentials::table
        .find(id)
        .select(CredentialRow::as_select())
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| Error::not_found("credential"))?;
    let org: String = vcsstar_db::schema::orgs::table
        .find(row.org_id)
        .select(vcsstar_db::schema::orgs::name)
        .first(&mut conn)?;
    let repo: Option<String> = match row.repo_id {
        Some(r) => repos::table
            .find(r)
            .select(repos::name)
            .first(&mut conn)
            .optional()?,
        None => None,
    };
    authorize_cred_target(app, &mut conn, ctx, &org, repo.as_deref())?;
    diesel::delete(credentials::table.find(id)).execute(&mut conn)?;
    Ok(WireOk { ok: true })
}

pub fn reconcile(app: &App, ctx: &ReqCtx, req: GetRepoRequest) -> Result<ReconcileReport, Error> {
    let (mut conn, org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Maintain)?;
    let started = now_ms();
    let outcome = mirror::reconcile(app, &mut conn, repo.id);
    let (refs_checked, pushed, drift, mut error) = match outcome {
        Ok(o) => {
            for id in &o.scheduled {
                if let Err(e) = mirror::submit_action(app, &org.name, &repo.name, *id, 0) {
                    tracing::warn!(error = %e.message, "reconcile: task submit failed");
                }
            }
            (o.refs_checked, o.scheduled.len(), o.drift, None)
        }
        Err(e) => (0, 0, vec![], Some(e.message)),
    };
    // GitHub's pull requests, reviews, comments, and statuses, by asking.
    let mut pulls = None;
    if let Some(state) = mirror::state_for(&mut conn, repo.id)? {
        match crate::github_sync::reconcile_pulls(
            app,
            &mut conn,
            &org,
            &repo,
            &state,
            crate::github_sync::PULL_SYNC_BUDGET,
        ) {
            Ok(o) => pulls = Some(o),
            Err(e) => {
                error = Some(match error {
                    Some(prev) => format!("{prev}; pull sync: {}", e.message),
                    None => format!("pull sync: {}", e.message),
                })
            }
        }
    }
    Ok(ReconcileReport {
        repo: vcs::repo_ref(&org, &repo),
        started_at: started,
        finished_at: now_ms(),
        refs_checked: refs_checked as u64,
        refs_pushed: pushed as u64,
        drift_refs: drift,
        error,
        pulls_checked: pulls.as_ref().map(|p| p.checked as u64),
        pulls_changed: pulls.as_ref().map(|p| p.changed as u64),
        pulls_complete: pulls.as_ref().map(|p| p.complete),
    })
}

/// Start the history import of a repository again from the first GitHub
/// pull request. Imported rows stay and match; the worker does the rest.
pub fn backfill(app: &App, ctx: &ReqCtx, req: GetRepoRequest) -> Result<MirrorStatus, Error> {
    let (mut conn, org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Admin)?;
    let Some(_) = mirror::state_for(&mut conn, repo.id)? else {
        return Err(Error::new(
            Code::PreconditionFailed,
            "this repository has no mirror; configure or import one first",
        ));
    };
    crate::github_sync::restart_backfill(&mut conn, repo.id)?;
    let s = mirror::state_for(&mut conn, repo.id)?
        .ok_or_else(|| Error::internal("mirror state vanished"))?;
    status_wire(&mut conn, &org, &repo, &s)
}

/// Where the instance's GitHub Apps are installed. An instance admin sees
/// every installation; anyone else sees those whose owner is the
/// `github_owner` of an organization they administer.
pub fn list_github_installations(
    app: &App,
    ctx: &ReqCtx,
    _req: vcsstar_types::Empty,
) -> Result<GithubInstallationList, Error> {
    let p = ctx.require()?;
    p.require_scope("mirror:manage")?;
    let mut conn = app.conn()?;
    let admin = authz::is_instance_admin(app, p);
    let owned: Vec<OrgRow> = vcsstar_db::schema::orgs::table
        .filter(vcsstar_db::schema::orgs::github_owner.is_not_null())
        .select(OrgRow::as_select())
        .load(&mut conn)?;
    let mut installations = Vec::new();
    let mut errors = Vec::new();
    for cfg in &app.config.github_apps {
        let jwt = match crate::github_app::sign_jwt(&cfg.app_id, &cfg.private_key) {
            Ok(j) => j,
            Err(e) => {
                errors.push(format!("App {}: {}", cfg.app_id, e.message));
                continue;
            }
        };
        let mut page = 1u32;
        loop {
            let (items, more) = match crate::github::list_app_installations(
                &app.config.github_api_base,
                &jwt,
                page,
            ) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("App {}: {}", cfg.app_id, e.message()));
                    break;
                }
            };
            for j in &items {
                let t = |ptr: &str| j.pointer(ptr).and_then(serde_json::Value::as_str);
                let Some(owner) = t("/account/login").map(str::to_string) else {
                    continue;
                };
                let mut orgs_here = Vec::new();
                for o in owned.iter().filter(|o| {
                    o.github_owner
                        .as_deref()
                        .is_some_and(|g| g.eq_ignore_ascii_case(&owner))
                }) {
                    if admin
                        || authz::require_org_role(app, &mut conn, o, p, OrgRole::Admin).is_ok()
                    {
                        orgs_here.push(o.name.clone());
                    }
                }
                if !admin && orgs_here.is_empty() {
                    continue;
                }
                installations.push(GithubInstallation {
                    app_id: cfg.app_id.clone(),
                    app_slug: cfg
                        .slug
                        .clone()
                        .or_else(|| t("/app_slug").map(str::to_string)),
                    installation_id: j.get("id").and_then(serde_json::Value::as_u64).unwrap_or(0),
                    owner,
                    owner_type: t("/account/type").unwrap_or("User").to_string(),
                    repository_selection: t("/repository_selection").unwrap_or("all").to_string(),
                    vcsstar_orgs: orgs_here,
                });
            }
            if !more {
                break;
            }
            page += 1;
        }
    }
    installations.sort_by_key(|a| a.owner.to_lowercase());
    Ok(GithubInstallationList {
        installations,
        errors,
    })
}

/// This node's view of GitHub: lane breakers, rate limits, and the size of
/// the mirror backlog across every repository.
pub fn health(app: &App, ctx: &ReqCtx, _req: vcsstar_types::Empty) -> Result<GithubHealth, Error> {
    ctx.require()?;
    let mut conn = app.conn()?;
    let (lanes, rate_limits) = crate::github_health::snapshot();
    let pending: i64 = mirror_actions::table
        .filter(mirror_actions::state.eq("pending"))
        .count()
        .get_result(&mut conn)?;
    let blocked: i64 = mirror_actions::table
        .filter(mirror_actions::state.eq("blocked"))
        .count()
        .get_result(&mut conn)?;
    let oldest: Option<chrono::DateTime<Utc>> = mirror_actions::table
        .filter(mirror_actions::state.eq("pending"))
        .select(diesel::dsl::min(mirror_actions::created_at))
        .first(&mut conn)?;
    Ok(GithubHealth {
        lanes,
        rate_limits,
        pending_actions: pending.max(0) as u64,
        blocked_actions: blocked.max(0) as u64,
        oldest_pending_at: oldest.map(to_ms),
    })
}

/// The GitHub repositories under the organization's `github_owner`, as
/// the instance App's installation (or the organization credential, or
/// the caller's own link) sees them, with the vcsstar repository that
/// already mirrors each one.
pub fn list_github_repos(
    app: &App,
    ctx: &ReqCtx,
    req: ListGithubReposRequest,
) -> Result<GithubRepoPage, Error> {
    let p = ctx.require()?;
    p.require_scope("mirror:manage")?;
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let owner = org.github_owner.clone().ok_or_else(|| {
        Error::new(
            Code::PreconditionFailed,
            "set github_owner on the organization first (Orgs/update)",
        )
    })?;
    let page: u32 = req
        .page
        .as_ref()
        .and_then(|p| p.cursor.as_deref())
        .map(|c| c.parse::<u32>().map_err(|_| Error::invalid("cursor")))
        .transpose()?
        .unwrap_or(1)
        .max(1);
    let per_page = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(100)
        .clamp(1, 100) as usize;
    let (token, installation) =
        cred::token_for_owner(app, &mut conn, org.id, &owner, Some(p.identity.id))?;
    let gh = crate::github::Client::new(&app.config.github_api_base, &token.value)
        .with_label(&token.label());
    if let Some(wait) = gh.rate_hold() {
        return Err(Error::new(
            Code::ResourceExhausted,
            format!(
                "GitHub rate limit reserve reached for {}; try again in {} s",
                token.label(),
                wait.as_secs()
            ),
        )
        .with_detail(format!("retry_after_s={}", wait.as_secs())));
    }
    let (items, more) = match &token.source {
        cred::TokenSource::InstanceApp { .. } => gh.list_installation_repos(page, per_page)?,
        _ => gh.list_owner_repos(&owner, page, per_page)?,
    };
    // Which GitHub repositories this organization already mirrors.
    let mirrored: Vec<(RepoRow, StateRow)> = repos::table
        .inner_join(mirror_state::table)
        .filter(repos::org_id.eq(org.id))
        .select((RepoRow::as_select(), StateRow::as_select()))
        .load(&mut conn)?;
    let mut out = Vec::with_capacity(items.len());
    for j in &items {
        let full = j
            .get("full_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        if full.is_empty() {
            continue;
        }
        // An installation lists every repository it covers; keep the owner's.
        if !full
            .split('/')
            .next()
            .is_some_and(|o| o.eq_ignore_ascii_case(&owner))
        {
            continue;
        }
        let b = |k: &str| {
            j.get(k)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        let t = |k: &str| {
            j.get(k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        let here = mirrored
            .iter()
            .find(|(_, st)| st.github_repo.eq_ignore_ascii_case(&full));
        out.push(GithubRepoInfo {
            mirrored_as: here.map(|(r, _)| r.name.clone()),
            import_state: here
                .and_then(|(r, st)| mirror_import::state(r, st))
                .map(str::to_string),
            full_name: full,
            description: t("description").filter(|d| !d.is_empty()),
            private: b("private"),
            archived: b("archived"),
            fork: b("fork"),
            default_branch: t("default_branch").unwrap_or_else(|| "main".into()),
            html_url: t("html_url").unwrap_or_default(),
            pushed_at: t("pushed_at")
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| to_ms(d.with_timezone(&Utc))),
        });
    }
    Ok(GithubRepoPage {
        repos: out,
        next_cursor: more.then(|| (page + 1).to_string()),
        source: token.label(),
        installation_id: installation.map(|i| i.max(0) as u64),
    })
}

/// Onboard an existing GitHub repository: make the vcsstar repository
/// (paused, mirror not enabled) and queue its import. A mirror worker
/// fetches every branch and tag and enables the mirror
/// (`mirror_import::run`); `status` reports `import_state`. A request for
/// a repository whose import did not finish (or failed) queues it again.
pub fn import_repo(app: &App, ctx: &ReqCtx, req: ImportRequest) -> Result<Repo, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    p.require_scope("mirror:manage")?;
    check_name("repository", &req.name)?;
    if !req.github_repo.contains('/') {
        return Err(Error::invalid("github_repo must be owner/name"));
    }
    let mode = req.mode.as_deref().unwrap_or("authoritative_vcsstar");
    if !mirror::MODES.contains(&mode) {
        return Err(Error::invalid(format!(
            "mode must be one of {:?}",
            mirror::MODES
        )));
    }
    let visibility = req.visibility.as_deref().unwrap_or("public");
    if !repo_svc::VISIBILITIES.contains(&visibility) {
        return Err(Error::invalid("visibility"));
    }
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let repo = queue_import(
        app,
        &mut conn,
        p,
        &ImportRow {
            org: &org,
            name: &req.name,
            github_repo: &req.github_repo,
            mode,
            visibility,
        },
    )?;
    Ok(repo_svc::to_wire(app, &org, &repo))
}

/// Queue the import of every GitHub repository under the organization's
/// `github_owner` with no mirror here yet. Lists GitHub (all pages) in the
/// request; the imports run in the mirror workers.
pub fn import_all(
    app: &App,
    ctx: &ReqCtx,
    req: ImportAllRequest,
) -> Result<ImportAllResult, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    p.require_scope("mirror:manage")?;
    let mode = req.mode.as_deref().unwrap_or("import_drift");
    if !mirror::MODES.contains(&mode) {
        return Err(Error::invalid(format!(
            "mode must be one of {:?}",
            mirror::MODES
        )));
    }
    let skip_archived = req.skip_archived.unwrap_or(false);
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let owner = org.github_owner.clone().ok_or_else(|| {
        Error::new(
            Code::PreconditionFailed,
            "set github_owner on the organization first (Orgs/update)",
        )
    })?;
    let (token, _) = cred::token_for_owner(app, &mut conn, org.id, &owner, Some(p.identity.id))?;
    let gh = crate::github::Client::new(&app.config.github_api_base, &token.value)
        .with_label(&token.label());
    let mut items = Vec::new();
    for page in 1..=50u32 {
        let (batch, more) = match &token.source {
            cred::TokenSource::InstanceApp { .. } => gh.list_installation_repos(page, 100)?,
            _ => gh.list_owner_repos(&owner, page, 100)?,
        };
        items.extend(batch);
        if !more {
            break;
        }
    }
    let mut out = ImportAllResult {
        queued: Vec::new(),
        skipped: Vec::new(),
    };
    for j in &items {
        let t = |k: &str| j.get(k).and_then(serde_json::Value::as_str).unwrap_or("");
        let b = |k: &str| {
            j.get(k)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        let full = t("full_name").to_string();
        let Some((o, name)) = full.split_once('/') else {
            continue;
        };
        if !o.eq_ignore_ascii_case(&owner) {
            continue;
        }
        if skip_archived && b("archived") {
            out.skipped.push(format!("{full}: archived"));
            continue;
        }
        let row = ImportRow {
            org: &org,
            name,
            github_repo: &full,
            mode,
            visibility: if b("private") { "private" } else { "public" },
        };
        if let Err(e) = check_name("repository", name) {
            out.skipped.push(format!("{full}: {}", e.message));
            continue;
        }
        match queue_import(app, &mut conn, p, &row) {
            Ok(_) => out.queued.push(full),
            Err(e) if e.code == Code::AlreadyExists => {}
            Err(e) => out.skipped.push(format!("{full}: {}", e.message)),
        }
    }
    tracing::info!(org = %org.name, queued = out.queued.len(), skipped = out.skipped.len(), "import-all queued");
    Ok(out)
}

/// Change how often the pull-based passes run for one repository.
pub fn set_schedule(
    app: &App,
    ctx: &ReqCtx,
    req: SetMirrorScheduleRequest,
) -> Result<MirrorStatus, Error> {
    let (mut conn, org, repo) =
        open_admin(app, ctx, &req.repo.org, &req.repo.name, RepoRole::Admin)?;
    let p = ctx.require()?;
    p.require_scope("mirror:manage")?;
    let check = |v: Option<u64>, what: &str| -> Result<Option<Option<i32>>, Error> {
        match v {
            None => Ok(None),
            Some(0) => Ok(Some(None)),
            Some(s) if (60..=604_800).contains(&s) => Ok(Some(Some(s as i32))),
            Some(_) => Err(Error::invalid(format!(
                "{what} must be 0 or 60 to 604800 seconds"
            ))),
        }
    };
    let pull = check(req.pull_sync_interval_s, "pull_sync_interval_s")?;
    let refs_every = check(req.ref_reconcile_interval_s, "ref_reconcile_interval_s")?;
    let state = mirror::state_for(&mut conn, repo.id)?
        .ok_or_else(|| Error::new(Code::PreconditionFailed, "mirror not configured"))?;
    conn.transaction::<(), Error, _>(|conn| {
        if let Some(v) = pull {
            diesel::update(mirror_state::table.find(repo.id))
                .set(mirror_state::pull_sync_interval_s.eq(v))
                .execute(conn)?;
        }
        if let Some(v) = refs_every {
            diesel::update(mirror_state::table.find(repo.id))
                .set(mirror_state::ref_reconcile_interval_s.eq(v))
                .execute(conn)?;
        }
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_repo(&org.name, &repo.name),
                kind: "mirror_configured",
                repo_id: Some(repo.id),
                org_id: Some(org.id),
                actor_id: Some(p.identity.id),
                actor: Some(p.to_wire()),
                repo: Some(vcs::repo_ref(&org, &repo)),
                org: Some(org.name.clone()),
                subject: None,
                payload: EventPayload::Variant5(SettingsPayload {
                    changed: vec!["schedule".into()],
                }),
            },
        )?;
        Ok(())
    })?;
    let _ = state;
    let s = mirror::state_for(&mut conn, repo.id)?
        .ok_or_else(|| Error::new(Code::PreconditionFailed, "mirror not configured"))?;
    status_wire(&mut conn, &org, &repo, &s)
}

/// What an import request writes for a new imported repository.
struct ImportRow<'a> {
    org: &'a OrgRow,
    name: &'a str,
    github_repo: &'a str,
    mode: &'a str,
    visibility: &'a str,
}

/// Make the row (or take the unfinished one) and submit the import task.
/// A finished import of the name answers `AlreadyExists`.
fn queue_import(
    app: &App,
    conn: &mut vcsstar_db::DbConn,
    p: &crate::principal::Principal,
    row: &ImportRow<'_>,
) -> Result<RepoRow, Error> {
    let now = Utc::now();
    // An import that did not finish (stopped, failed, or still queued)
    // keeps its row: queue it again for this requester.
    let stub: Option<RepoRow> = repos::table
        .inner_join(mirror_state::table)
        .filter(repos::org_id.eq(row.org.id))
        .filter(repos::name.eq(row.name))
        .filter(repos::mirror_enabled.eq(false))
        .filter(mirror_state::github_repo.eq(row.github_repo))
        .filter(mirror_state::import_done_at.is_null())
        .select(RepoRow::as_select())
        .first(conn)
        .optional()?;
    let stub = match stub {
        Some(r) if refs::load(conn, r.id)?.is_empty() => Some(r),
        _ => None,
    };
    let repo = match stub {
        Some(r) => {
            let state = mirror::state_for(conn, r.id)?;
            let running = state.as_ref().is_some_and(|s| {
                s.import_requested_at.is_some() && s.import_attempts < mirror_import::MAX_ATTEMPTS
            });
            if running {
                // Still queued: nothing to change.
                return Ok(r);
            }
            diesel::update(mirror_state::table.find(r.id))
                .set((
                    mirror_state::import_requested_by.eq(p.identity.id),
                    mirror_state::import_requested_at.eq(now),
                    mirror_state::import_attempts.eq(0),
                    mirror_state::import_claimed_at.eq(None::<chrono::DateTime<Utc>>),
                ))
                .execute(conn)?;
            tracing::info!(repo = %row.github_repo, "import queued again");
            r
        }
        None => create_import_row(conn, p, row, now)?,
    };
    if let Err(e) = mirror_import::submit(app, repo.id, now) {
        // The worker's sweep submits it again (`mirror_import::resubmit_lost`).
        tracing::warn!(repo = %row.github_repo, error = %e.message, "import task not submitted; the sweep retries");
    }
    Ok(repo)
}

/// The repository row and its paused mirror state, before any fetch.
fn create_import_row(
    conn: &mut vcsstar_db::DbConn,
    p: &crate::principal::Principal,
    row: &ImportRow<'_>,
    now: chrono::DateTime<Utc>,
) -> Result<RepoRow, Error> {
    let repo_id = vcsstar_db::new_id();
    // The row mirrors what GitHub has: the event carries origin `github`,
    // so the dispatcher sends no settings back (an empty description would
    // otherwise blank GitHub's). The worker sets GitHub's default branch
    // and description.
    outbox::with_origin("github", || {
        conn.transaction::<RepoRow, Error, _>(|conn| {
            let new_row = NewRepo {
                id: repo_id,
                org_id: row.org.id,
                name: row.name,
                visibility: row.visibility,
                vcs_type: "git",
                default_branch: "main",
                description: None,
            };
            let repo: RepoRow = diesel::insert_into(repos::table)
                .values(&new_row)
                .returning(RepoRow::as_returning())
                .get_result(conn)?;
            diesel::insert_into(mirror_state::table)
                .values((
                    mirror_state::repo_id.eq(repo.id),
                    mirror_state::github_repo.eq(row.github_repo),
                    mirror_state::mode.eq(row.mode),
                    mirror_state::paused.eq(true),
                    mirror_state::import_requested_by.eq(p.identity.id),
                    mirror_state::import_requested_at.eq(now),
                ))
                .execute(conn)?;
            outbox::emit(
                conn,
                Emit {
                    lane: outbox::lane_for_repo(&row.org.name, &repo.name),
                    kind: "repo_created",
                    repo_id: Some(repo.id),
                    org_id: Some(row.org.id),
                    actor_id: Some(p.identity.id),
                    actor: Some(p.to_wire()),
                    repo: Some(vcs::repo_ref(row.org, &repo)),
                    org: Some(row.org.name.clone()),
                    subject: None,
                    payload: EventPayload::Variant5(SettingsPayload {
                        changed: vec!["created".into(), format!("import:{}", row.github_repo)],
                    }),
                },
            )?;
            Ok(repo)
        })
    })
}
