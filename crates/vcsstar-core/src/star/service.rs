//! Public CSIL operations for connections, conflict policies, and links.
use super::{engine, local, remote, store};
use crate::{
    authz::{self, RepoRole},
    mirror,
    principal::ReqCtx,
    App, Code, Error,
};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Bool, Nullable, Text, Uuid as SqlUuid};
use uuid::Uuid;
use vcsstar_db::models::{Org, Repo};
use vcsstar_types as t;

fn open(
    app: &App,
    ctx: &ReqCtx,
    repo: &t::RepoRef,
    role: RepoRole,
) -> Result<(vcsstar_db::DbConn, Org, Repo), Error> {
    let p = ctx.require()?;
    p.require_scope(if role == RepoRole::Read {
        "repo:read"
    } else {
        "mirror:manage"
    })?;
    let mut conn = app.conn()?;
    let (org, repo, actual) = authz::load_repo_visible(app, &mut conn, ctx, &repo.org, &repo.name)?;
    authz::require_repo_role(actual, role)?;
    Ok((conn, org, repo))
}
fn uuid(s: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(s).map_err(|_| Error::invalid("invalid id"))
}
fn owned(conn: &mut PgConnection, repo: Uuid, id: &str) -> Result<store::Connection, Error> {
    let c = store::connection(conn, uuid(id)?)?;
    if c.repo_id != repo {
        return Err(Error::not_found("connection"));
    }
    Ok(c)
}
fn wire(
    app: &App,
    conn: &mut PgConnection,
    c: &store::Connection,
) -> Result<t::SyncConnection, Error> {
    let replicas = store::replicas(conn, c.id)?;
    Ok(t::SyncConnection {
        id: c.id.to_string(),
        provider: c.provider.clone(),
        host: c.host.clone(),
        project: c.project.clone(),
        paused: c.paused,
        inbound_enabled: c.inbound_enabled,
        outbound_enabled: c.outbound_enabled,
        automatic_webhook: c.automatic_webhook,
        setup_done: c.setup_done,
        webhook_url: c.webhook_url(app),
        last_success_at: c.last_success_at.map(crate::to_ms),
        last_error: c.last_error.clone(),
        pending: u64::from(c.requested > c.completed),
        blocked: replicas.iter().filter(|r| r.outcome == "blocked").count() as u64,
        skipped: replicas.iter().filter(|r| r.outcome == "skipped").count() as u64,
    })
}
pub fn list(app: &App, ctx: &ReqCtx, req: t::GetRepoRequest) -> Result<t::SyncConnections, Error> {
    let (mut conn, _, repo) = open(app, ctx, &req.repo, RepoRole::Read)?;
    Ok(t::SyncConnections {
        connections: store::connections(&mut conn, repo.id)?
            .iter()
            .map(|c| wire(app, &mut conn, c))
            .collect::<Result<_, _>>()?,
    })
}
pub fn connect(
    app: &App,
    ctx: &ReqCtx,
    req: t::ConnectRemoteRequest,
) -> Result<t::SyncConnection, Error> {
    let (mut conn, org, repo) = open(app, ctx, &req.repo, RepoRole::Admin)?;
    let (provider, host) = mirror::target(Some(&req.provider), Some(&req.host))?;
    mirror::check_project(&provider, &req.project)?;
    // Preserve an old mirror when attaching a second destination before
    // the worker's next adoption pass.
    engine::adopt_legacy(app, &mut conn, Some(repo.id))?;
    let id = Uuid::now_v7();
    let secret = mirror::cred::seal(
        app,
        format!("{}{}", Uuid::new_v4(), Uuid::new_v4()).as_bytes(),
    )?;
    let c=conn.transaction::<store::Connection,Error,_>(|conn|{
        let c=diesel::sql_query("INSERT INTO sync_connections(id,repo_id,provider,host,project,create_remote,automatic_webhook,webhook_secret,inbound_enabled,outbound_enabled) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *")
            .bind::<SqlUuid,_>(id).bind::<SqlUuid,_>(repo.id).bind::<Text,_>(&provider).bind::<Text,_>(&host).bind::<Text,_>(&req.project)
            .bind::<Bool,_>(req.create_remote.unwrap_or(false)).bind::<Bool,_>(req.automatic_webhook.unwrap_or(false)).bind::<Binary,_>(secret).bind::<Bool,_>(req.inbound_enabled.unwrap_or(false)).bind::<Bool,_>(req.outbound_enabled.unwrap_or(false)).get_result(conn)?;
        diesel::sql_query("INSERT INTO sync_policy(repo_id,resolver_identity) VALUES($1,$2) ON CONFLICT DO NOTHING").bind::<SqlUuid,_>(repo.id).bind::<SqlUuid,_>(ctx.require()?.identity.id).execute(conn)?;
        diesel::update(vcsstar_db::schema::repos::table.find(repo.id)).set(vcsstar_db::schema::repos::mirror_enabled.eq(true)).execute(conn)?;
        local::emit(conn,&org,&repo,"sync_connected",&id.to_string(),ctx.actor_id())?;
        Ok(c)
    })?;
    wire(app, &mut conn, &c)
}
pub fn update(
    app: &App,
    ctx: &ReqCtx,
    req: t::UpdateConnectionRequest,
) -> Result<t::SyncConnection, Error> {
    let (mut conn, org, repo) = open(app, ctx, &req.repo, RepoRole::Admin)?;
    let c = owned(&mut conn, repo.id, &req.connection_id)?;
    let c=conn.transaction::<store::Connection,Error,_>(|conn|{
        let c=diesel::sql_query("UPDATE sync_connections SET paused=COALESCE($2,paused),inbound_enabled=COALESCE($4,inbound_enabled),outbound_enabled=COALESCE($5,outbound_enabled),automatic_webhook=COALESCE($3,automatic_webhook),setup_done=CASE WHEN $3 IS NOT NULL AND $3<>automatic_webhook THEN false ELSE setup_done END,requested=requested+1,poll_after=now() WHERE id=$1 RETURNING *")
            .bind::<SqlUuid,_>(c.id).bind::<Nullable<Bool>,_>(req.paused).bind::<Nullable<Bool>,_>(req.automatic_webhook).bind::<Nullable<Bool>,_>(req.inbound_enabled).bind::<Nullable<Bool>,_>(req.outbound_enabled).get_result(conn)?;
        local::emit(conn,&org,&repo,"sync_configured",&req.connection_id,ctx.actor_id())?;Ok(c)
    })?;
    wire(app, &mut conn, &c)
}
pub fn sync_now(app: &App, ctx: &ReqCtx, req: t::SyncConnectionRef) -> Result<t::Ok, Error> {
    let (mut conn, org, repo) = open(app, ctx, &req.repo, RepoRole::Maintain)?;
    let c = owned(&mut conn, repo.id, &req.connection_id)?;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::sql_query(
            "UPDATE sync_connections SET requested=requested+1,poll_after=now() WHERE id=$1",
        )
        .bind::<SqlUuid, _>(c.id)
        .execute(conn)?;
        local::emit(
            conn,
            &org,
            &repo,
            "sync_requested",
            &req.connection_id,
            ctx.actor_id(),
        )
    })?;
    Ok(t::Ok { ok: true })
}
pub fn webhook_secret(
    app: &App,
    ctx: &ReqCtx,
    req: t::SyncConnectionRef,
) -> Result<t::SyncWebhookSecret, Error> {
    let (mut conn, _, repo) = open(app, ctx, &req.repo, RepoRole::Admin)?;
    let c = owned(&mut conn, repo.id, &req.connection_id)?;
    Ok(t::SyncWebhookSecret {
        url: c.webhook_url(app),
        secret: mirror::cred::open(app, &c.webhook_secret)?,
    })
}
fn policy_wire(conn: &mut PgConnection, repo: Uuid) -> Result<t::SyncPolicy, Error> {
    let p = store::policy(conn, repo)?;
    let resolver_identity = p
        .as_ref()
        .and_then(|p| p.resolver_identity)
        .map(|id| {
            use vcsstar_db::schema::identities;
            let (user, domain): (Uuid, String) = identities::table
                .find(id)
                .select((identities::lk_user_id, identities::lk_domain))
                .first(conn)?;
            Ok::<_, Error>(format!("{user}@{domain}"))
        })
        .transpose()?;
    Ok(t::SyncPolicy {
        preferred_source: p.as_ref().and_then(|p| p.preferred_source.clone()),
        resolver_identity,
        resolver_team: p.and_then(|p| p.resolver_team).map(|id| id.to_string()),
    })
}
pub fn get_policy(app: &App, ctx: &ReqCtx, req: t::GetRepoRequest) -> Result<t::SyncPolicy, Error> {
    let (mut conn, _, repo) = open(app, ctx, &req.repo, RepoRole::Read)?;
    policy_wire(&mut conn, repo.id)
}
pub fn set_policy(
    app: &App,
    ctx: &ReqCtx,
    req: t::SetSyncPolicyRequest,
) -> Result<t::SyncPolicy, Error> {
    let (mut conn, org, repo) = open(app, ctx, &req.repo, RepoRole::Admin)?;
    let p = &req.policy;
    if let Some(source) = &p.preferred_source {
        if source != "local" {
            owned(&mut conn, repo.id, source)?;
        }
    }
    let identity = p
        .resolver_identity
        .as_deref()
        .map(|value| resolver_id(&mut conn, value))
        .transpose()?;
    let team = p.resolver_team.as_deref().map(uuid).transpose()?;
    if p.preferred_source.is_none() && identity.is_none() && team.is_none() {
        return Err(Error::invalid(
            "select a preferred source or a resolver identity/team",
        ));
    }
    if p.preferred_source.is_some() && (identity.is_some() || team.is_some()) {
        return Err(Error::invalid(
            "choose a preferred source or resolvers, not both",
        ));
    }
    if let Some(id) = team {
        use vcsstar_db::schema::teams;
        if teams::table
            .find(id)
            .filter(teams::org_id.eq(org.id))
            .select(teams::id)
            .first::<Uuid>(&mut conn)
            .optional()?
            .is_none()
        {
            return Err(Error::invalid(
                "resolver team must belong to this organization",
            ));
        }
    }
    conn.transaction::<(),Error,_>(|conn|{
        diesel::sql_query("INSERT INTO sync_policy(repo_id,preferred_source,resolver_identity,resolver_team) VALUES($1,$2,$3,$4) ON CONFLICT(repo_id) DO UPDATE SET preferred_source=$2,resolver_identity=$3,resolver_team=$4")
            .bind::<SqlUuid,_>(repo.id).bind::<Nullable<Text>,_>(p.preferred_source.as_deref()).bind::<Nullable<SqlUuid>,_>(identity).bind::<Nullable<SqlUuid>,_>(team).execute(conn)?;
        store::wake(conn,repo.id)?;local::emit(conn,&org,&repo,"sync_policy_changed","policy",ctx.actor_id())
    })?;
    policy_wire(&mut conn, repo.id)
}
pub fn conflicts(
    app: &App,
    ctx: &ReqCtx,
    req: t::GetRepoRequest,
) -> Result<t::SyncConflicts, Error> {
    let (mut conn, _, repo) = open(app, ctx, &req.repo, RepoRole::Read)?;
    Ok(t::SyncConflicts {
        conflicts: store::conflicts(&mut conn, repo.id)?
            .into_iter()
            .map(|c| t::SyncConflict {
                id: c.id.to_string(),
                connection_id: c.connection_id.to_string(),
                object_key: c.object_key,
                local_json: c.local_value.unwrap_or(serde_json::Value::Null).to_string(),
                remote_json: c
                    .remote_value
                    .unwrap_or(serde_json::Value::Null)
                    .to_string(),
                created_at: crate::to_ms(c.created_at),
            })
            .collect(),
    })
}
pub fn resolve(
    app: &App,
    ctx: &ReqCtx,
    req: t::ResolveSyncConflictRequest,
) -> Result<t::Ok, Error> {
    let (mut conn, org, repo) = open(app, ctx, &req.repo, RepoRole::Read)?;
    let actor = ctx.require()?.identity.id;
    let p = store::policy(&mut conn, repo.id)?;
    let allowed = match &p {
        Some(p) => {
            p.resolver_identity == Some(actor)
                || if let Some(team) = p.resolver_team {
                    use vcsstar_db::schema::team_members;
                    team_members::table
                        .find((team, actor))
                        .select(team_members::identity_id)
                        .first::<Uuid>(&mut conn)
                        .optional()?
                        .is_some()
                } else {
                    false
                }
        }
        None => {
            let (_, _, role) =
                authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
            role >= RepoRole::Admin
        }
    };
    if !allowed {
        return Err(Error::forbidden(
            "only the assigned resolver identity or team can resolve this conflict",
        ));
    }
    if !["local", "remote"].contains(&req.choice.as_str()) {
        return Err(Error::invalid("choice must be local or remote"));
    }
    let id = uuid(&req.conflict_id)?;
    conn.transaction::<(),Error,_>(|conn|{
        let conflict:store::Conflict=diesel::sql_query("SELECT * FROM sync_conflicts WHERE id=$1 AND repo_id=$2 AND resolved_at IS NULL FOR UPDATE").bind::<SqlUuid,_>(id).bind::<SqlUuid,_>(repo.id).get_result(conn)?;
        let local_item=store::item(&conflict.local_value)?;
        let remote_item=store::item(&conflict.remote_value)?;
        let current=local::snapshot(conn,&repo)?.remove(&conflict.object_key);
        if current.as_ref().map(|i|&i.data)!=local_item.as_ref().map(|i|&i.data){return Err(Error::new(Code::Conflict,"the local item changed; sync again before resolving"));}
        if req.choice=="remote" && local_item.as_ref().is_some_and(|i|i.kind=="history") && remote_item.as_ref().is_some_and(|i|i.kind=="summary") {
            return Err(Error::invalid("status history is an audit record; change the status at its source and keep the local history record"));
        }
        if req.choice == "remote" && remote_item.as_ref().is_some_and(|i| i.kind != conflict.object_key.split(':').next().unwrap_or("") && local_item.as_ref().and_then(|local|remote::expand_projection(local,i)).is_none()) {
            return Err(Error::invalid("the destination has a summary; restore the full record in VCSStar and select the local version"));
        }
        // The worker reads both sides again and fetches objects before it
        // applies this choice. A later edit requires a new decision.
        diesel::sql_query("UPDATE sync_conflicts SET resolved_by=$2,resolution=$3 WHERE id=$1").bind::<SqlUuid,_>(id).bind::<SqlUuid,_>(actor).bind::<Text,_>(&req.choice).execute(conn)?;
        store::wake(conn,repo.id)?;local::emit(conn,&org,&repo,"sync_resolved",&conflict.object_key,Some(actor))
    })?;
    Ok(t::Ok { ok: true })
}
#[derive(QueryableByName)]
struct LinkRow {
    #[diesel(sql_type=Text)]
    provider: String,
    #[diesel(sql_type=Text)]
    host: String,
    #[diesel(sql_type=Text)]
    login: String,
    #[diesel(sql_type=Nullable<Text>)]
    broken_reason: Option<String>,
}
fn link_wire(r: LinkRow) -> t::ProviderLink {
    t::ProviderLink {
        provider: r.provider,
        host: r.host,
        login: r.login,
        broken_reason: r.broken_reason,
    }
}
pub fn link(
    app: &App,
    ctx: &ReqCtx,
    req: t::LinkProviderRequest,
) -> Result<t::ProviderLink, Error> {
    let p = ctx.require()?;
    link_permission(p)?;
    let (provider, host) = mirror::target(Some(&req.provider), Some(&req.host))?;
    if req.token.trim().is_empty() {
        return Err(Error::invalid("token is required"));
    }
    let c = store::Connection {
        id: Uuid::nil(),
        repo_id: Uuid::nil(),
        provider: provider.clone(),
        host: host.clone(),
        project: String::new(),
        paused: false,
        inbound_enabled: false,
        outbound_enabled: false,
        create_remote: false,
        automatic_webhook: false,
        setup_done: false,
        webhook_secret: vec![],
        requested: 0,
        completed: 0,
        poll_after: chrono::Utc::now(),
        last_success_at: None,
        last_error: None,
    };
    let client = remote::Client::new(app, &c, req.token.trim().into(), true);
    let user = client.call("GET", "/user", None)?;
    let remote_id = user
        .get("id")
        .ok_or_else(|| Error::invalid("provider returned no user id"))?
        .to_string();
    let login = user
        .get("login")
        .or_else(|| user.get("username"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::invalid("provider returned no login"))?;
    let sealed = mirror::cred::seal(app, req.token.trim().as_bytes())?;
    let mut conn = app.conn()?;
    let row=conn.transaction::<LinkRow,Error,_>(|conn| {
        let row=diesel::sql_query("INSERT INTO sync_links(identity_id,provider,host,remote_user_id,login,token) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(identity_id,provider,host) DO UPDATE SET remote_user_id=$4,login=$5,token=$6,broken_reason=NULL RETURNING *")
        .bind::<SqlUuid,_>(p.identity.id).bind::<Text,_>(&provider).bind::<Text,_>(&host).bind::<Text,_>(&remote_id).bind::<Text,_>(login).bind::<Binary,_>(sealed).get_result(conn)?;
        let shadow_user=Uuid::new_v5(&Uuid::NAMESPACE_URL,format!("https://{}/users/{remote_id}",host).as_bytes());
        #[derive(QueryableByName)] struct Shadow { #[diesel(sql_type=SqlUuid)] id:Uuid }
        let shadow:Option<Shadow>=diesel::sql_query("SELECT id FROM identities WHERE lk_user_id=$1 AND lk_domain=$2").bind::<SqlUuid,_>(shadow_user).bind::<Text,_>(&host).get_result(conn).optional()?;
        let shadow=if provider=="github" {
            diesel::sql_query("SELECT identity_id AS id FROM github_identities WHERE github_id::text=$1").bind::<Text,_>(&remote_id).get_result::<Shadow>(conn).optional()?
        } else { shadow };
        if let Some(shadow)=shadow {
            for (table,column) in [("pull_requests","author_identity_id"),("pr_comments","author"),("pr_reviews","reviewer"),("statuses","reporter")] {
                diesel::sql_query(format!("UPDATE {table} SET {column}=$2 WHERE {column}=$1")).bind::<SqlUuid,_>(shadow.id).bind::<SqlUuid,_>(p.identity.id).execute(conn)?;
            }
            // Keep original labels and timestamps while using the linked identity.
            diesel::sql_query("UPDATE sync_objects SET payload=jsonb_set(payload,'{actor}',to_jsonb($2::text)) WHERE payload->>'actor'=$1").bind::<Text,_>(shadow.id.to_string()).bind::<Text,_>(p.identity.id.to_string()).execute(conn)?;
        }
        account_event(conn,p,"provider_linked",&format!("{provider}:{host}"))?;
        Ok(row)
    })?;
    Ok(link_wire(row))
}
pub fn links(app: &App, ctx: &ReqCtx, _req: t::Empty) -> Result<t::ProviderLinks, Error> {
    let p = ctx.require()?;
    let mut conn = app.conn()?;
    let rows: Vec<LinkRow> =
        diesel::sql_query("SELECT * FROM sync_links WHERE identity_id=$1 ORDER BY provider,host")
            .bind::<SqlUuid, _>(p.identity.id)
            .load(&mut conn)?;
    Ok(t::ProviderLinks {
        links: rows.into_iter().map(link_wire).collect(),
    })
}
pub fn unlink(app: &App, ctx: &ReqCtx, req: t::UnlinkProviderRequest) -> Result<t::Ok, Error> {
    let p = ctx.require()?;
    link_permission(p)?;
    let mut conn = app.conn()?;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::sql_query(
            "DELETE FROM sync_links WHERE identity_id=$1 AND provider=$2 AND host=$3",
        )
        .bind::<SqlUuid, _>(p.identity.id)
        .bind::<Text, _>(&req.provider)
        .bind::<Text, _>(req.host.to_ascii_lowercase())
        .execute(conn)?;
        account_event(
            conn,
            p,
            "provider_unlinked",
            &format!("{}:{}", req.provider, req.host),
        )
    })?;
    Ok(t::Ok { ok: true })
}
pub fn records(
    app: &App,
    ctx: &ReqCtx,
    req: t::SyncRecordsRequest,
) -> Result<t::SyncRecords, Error> {
    let (mut conn, _, repo) = open(app, ctx, &req.repo, RepoRole::Read)?;
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(100)
        .clamp(1, 200) as i64;
    let cursor = req
        .page
        .as_ref()
        .and_then(|p| p.cursor.as_deref())
        .unwrap_or("");
    let mut rows:Vec<store::Object>=diesel::sql_query("SELECT * FROM sync_objects WHERE repo_id=$1 AND object_key>$2 ORDER BY object_key LIMIT $3").bind::<SqlUuid,_>(repo.id).bind::<Text,_>(cursor).bind::<BigInt,_>(limit+1).load(&mut conn)?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().map(|r| r.object_key.clone())
    } else {
        None
    };
    Ok(t::SyncRecords {
        records: rows
            .into_iter()
            .map(|r| t::SyncRecord {
                object_key: r.object_key,
                payload_json: r.payload.to_string(),
                deleted: r.deleted,
                source: r.source,
            })
            .collect(),
        next_cursor,
    })
}

fn link_permission(p: &crate::principal::Principal) -> Result<(), Error> {
    use crate::principal::CredentialKind;
    if p.kind == CredentialKind::ActAs {
        return Err(Error::forbidden(
            "an act-as bearer cannot change provider account links",
        ));
    }
    if p.kind == CredentialKind::ApiToken {
        p.require_scope("tokens:manage")?;
    }
    Ok(())
}

fn resolver_id(conn: &mut PgConnection, value: &str) -> Result<Uuid, Error> {
    use vcsstar_db::schema::identities;
    if let Some((user, domain)) = value.rsplit_once('@') {
        identities::table
            .filter(identities::lk_user_id.eq(uuid(user)?))
            .filter(identities::lk_domain.eq(domain))
            .select(identities::id)
            .first(conn)
            .map_err(Error::from)
    } else {
        uuid(value)
    }
}
fn account_event(
    conn: &mut PgConnection,
    p: &crate::principal::Principal,
    kind: &str,
    subject: &str,
) -> Result<(), Error> {
    crate::outbox::emit(
        conn,
        crate::outbox::Emit {
            lane: crate::services::instance::LANE.into(),
            kind,
            repo_id: None,
            org_id: None,
            actor_id: Some(p.identity.id),
            actor: Some(p.to_wire()),
            repo: None,
            org: None,
            subject: Some(subject.into()),
            payload: t::EventPayload::Variant5(t::SettingsPayload {
                changed: vec![subject.into()],
            }),
        },
    )?;
    Ok(())
}
