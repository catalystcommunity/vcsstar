//! Mirror service: status, blocked list and replay, credentials, pause and
//! resume, configure, reconcile, import. The engine is `crate::mirror`.

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_db::models::{NewRepo, Org as OrgRow, Repo as RepoRow};
use vcsstar_db::schema::{credentials, mirror_actions, mirror_state, repos};
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_types::{
    BlockedAction, ConfigureMirrorRequest, Credential, CredentialRef, EventPayload, GetRepoRequest,
    ImportRequest, ListBlockedResponse, ListCredentialsRequest, ListCredentialsResponse,
    MirrorStatus, Ok as WireOk, ReconcileReport, ReplayRequest, ReplayResult, Repo,
    SetCredentialRequest, SettingsPayload,
};
use vcsstar_vcs::RevId;

use crate::authz::{self, OrgRole, RepoRole};
use crate::mirror::{self, cred, ActionRow, CredentialRow, StateRow};
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
    if req.auth_method == "github_app" {
        return Err(Error::not_implemented(
            "github_app credentials (use a pat for now)",
        ));
    }
    let ciphertext = if req.secret_ref == "stored" {
        let secret = req
            .secret
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::invalid("secret is required when secret_ref is stored"))?;
        Some(cred::seal(app, secret.trim().as_bytes())?)
    } else if let Some(var) = req.secret_ref.strip_prefix("env:") {
        if var.is_empty() || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::invalid("env: ref must name an environment variable"));
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
            credentials::secret_ciphertext.eq(ciphertext.clone()),
            credentials::enabled.eq(req.enabled.unwrap_or(true)),
            credentials::created_by.eq(ctx.actor_id()),
        ))
        .on_conflict((
            credentials::org_id,
            credentials::repo_id,
            credentials::provider,
            credentials::purpose,
        ))
        .do_update()
        .set((
            credentials::auth_method.eq(&req.auth_method),
            credentials::host.eq(&req.host),
            credentials::secret_ref.eq(&req.secret_ref),
            credentials::secret_ciphertext.eq(ciphertext),
            credentials::enabled.eq(req.enabled.unwrap_or(true)),
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
    let (refs_checked, pushed, drift, error) = match outcome {
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
    Ok(ReconcileReport {
        repo: vcs::repo_ref(&org, &repo),
        started_at: started,
        finished_at: now_ms(),
        refs_checked: refs_checked as u64,
        refs_pushed: pushed as u64,
        drift_refs: drift,
        error,
    })
}

/// Onboard an existing GitHub repository: create the vcsstar repo, fetch
/// every branch and tag, record them as refs and as already mirrored, and
/// configure the mirror in the requested mode.
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
    let token = cred::find(&mut conn, org.id, uuid::Uuid::nil(), "import_fetch")?
        .or(cred::find(
            &mut conn,
            org.id,
            uuid::Uuid::nil(),
            "mirror_git_push",
        )?)
        .map(|row| cred::value(app, &row))
        .transpose()?;
    // Learn the default branch and the refs from GitHub before any row exists.
    let scratch = tempfile::tempdir().map_err(Error::internal)?;
    let scratch_dir = scratch.path().join("probe.git");
    app.store.backend().init(&scratch_dir)?;
    let url = format!("https://github.com/{}.git", req.github_repo);
    let remote = mirror::ls_remote(&scratch_dir, &url, token.as_deref())?;
    let default_branch = token
        .as_deref()
        .and_then(|t| {
            crate::github::Client::new(&app.config.github_api_base, t)
                .get_repo(&req.github_repo)
                .ok()
                .flatten()
                .and_then(|r| r.default_branch)
        })
        .or_else(|| {
            remote
                .iter()
                .find(|(n, _)| n == "refs/heads/main" || n == "refs/heads/master")
                .map(|(n, _)| n.trim_start_matches("refs/heads/").to_string())
        })
        .unwrap_or_else(|| "main".into());
    let repo_id = vcsstar_db::new_id();
    let repo = conn.transaction::<RepoRow, Error, _>(|conn| {
        let row = NewRepo {
            id: repo_id,
            org_id: org.id,
            name: &req.name,
            visibility,
            vcs_type: "git",
            default_branch: &default_branch,
            description: None,
        };
        let repo: RepoRow = diesel::insert_into(repos::table)
            .values(&row)
            .returning(RepoRow::as_returning())
            .get_result(conn)?;
        diesel::insert_into(mirror_state::table)
            .values((
                mirror_state::repo_id.eq(repo.id),
                mirror_state::github_repo.eq(&req.github_repo),
                mirror_state::mode.eq(mode),
                mirror_state::paused.eq(true),
            ))
            .execute(conn)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_repo(&org.name, &repo.name),
                kind: "repo_created",
                repo_id: Some(repo.id),
                org_id: Some(org.id),
                actor_id: Some(p.identity.id),
                actor: Some(p.to_wire()),
                repo: Some(vcs::repo_ref(&org, &repo)),
                org: Some(org.name.clone()),
                subject: None,
                payload: EventPayload::Variant5(SettingsPayload {
                    changed: vec!["created".into(), format!("import:{}", req.github_repo)],
                }),
            },
        )?;
        Ok(repo)
    })?;
    // Fetch history into the cache, then record refs (CAS from absent) and
    // the mirrored shas so the first dispatch pushes nothing.
    let dir = app.store.ensure(repo.id)?;
    let fetch = |refspec: &str| mirror::fetch_ref(&dir, &url, token.as_deref(), refspec, refspec);
    for (name, _) in &remote {
        if let Err(e) = fetch(name) {
            return Err(Error::new(
                Code::Unavailable,
                format!("fetch {name}: {}", e.message()),
            ));
        }
    }
    let updates: Vec<RefUpdate> = remote
        .iter()
        .map(|(n, s)| RefUpdate {
            name: n.clone(),
            old: None,
            new: Some(RevId(s.clone())),
        })
        .collect();
    let want: Vec<RevId> = remote.iter().map(|(_, s)| RevId(s.clone())).collect();
    conn.transaction::<(), Error, _>(|conn| {
        vcs::capture(app, conn, repo.id, &dir, &[], &want)?;
        refs::apply(conn, repo.id, &updates, Some(p.identity.id))?;
        for (n, s) in &remote {
            diesel::insert_into(vcsstar_db::schema::mirror_refs::table)
                .values((
                    vcsstar_db::schema::mirror_refs::repo_id.eq(repo.id),
                    vcsstar_db::schema::mirror_refs::ref_.eq(n),
                    vcsstar_db::schema::mirror_refs::mirrored_sha.eq(s),
                    vcsstar_db::schema::mirror_refs::last_verified_at.eq(Utc::now()),
                ))
                .execute(conn)?;
        }
        diesel::update(mirror_state::table.find(repo.id))
            .set((
                mirror_state::paused.eq(false),
                mirror_state::last_full_reconcile_at.eq(Utc::now()),
            ))
            .execute(conn)?;
        diesel::update(repos::table.find(repo.id))
            .set(repos::mirror_enabled.eq(true))
            .execute(conn)?;
        Ok(())
    })?;
    let repo = repos::table
        .find(repo.id)
        .select(RepoRow::as_select())
        .first(&mut conn)?;
    Ok(repo_svc::to_wire(app, &org, &repo))
}
