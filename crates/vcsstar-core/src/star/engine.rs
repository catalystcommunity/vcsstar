//! Durable, connection-scoped reconciliation. Unrelated objects continue
//! when an item needs a resolver or destination permission.
use super::{decide, local, remote, store, text, Decision, Item};
use crate::{mirror, vcs, App, Code, Error};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Text, Uuid as SqlUuid};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const QUEUE: &str = "sync:connections";
#[derive(Debug, Serialize, Deserialize)]
pub struct Task {
    pub connection_id: Uuid,
    #[serde(default)]
    pub webhook: Option<Vec<u8>>,
}

pub fn schedule(app: &App) -> Result<(), Error> {
    let mut conn = app.conn()?;
    adopt_legacy(app, &mut conn, None)?;
    let rows:Vec<store::Connection>=diesel::sql_query("SELECT * FROM sync_connections WHERE NOT paused AND poll_after<=now() ORDER BY poll_after LIMIT 100").load(&mut conn)?;
    for c in rows {
        let epoch = chrono::Utc::now().timestamp() / 60;
        let mut payload = vec![];
        ciborium::ser::into_writer(
            &Task {
                connection_id: c.id,
                webhook: None,
            },
            &mut payload,
        )
        .map_err(Error::internal)?;
        match app.submit_task(
            QUEUE,
            &format!("sync:{}:{}:{epoch}", c.id, c.requested),
            payload,
            1,
        ) {
            Ok(_) => {}
            Err(e) if e.code == Code::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Existing configurations remain available through their old API. Once
/// import finishes, a connection takes over their sync work.
pub fn adopt_legacy(app: &App, conn: &mut PgConnection, only: Option<Uuid>) -> Result<(), Error> {
    if app.config.credential_key.is_none() {
        return Ok(());
    }
    use vcsstar_db::schema::{mirror_state, repos};
    let legacy: Vec<mirror::StateRow> = mirror_state::table
        .inner_join(repos::table)
        .filter(repos::mirror_enabled.eq(true))
        .select(mirror::StateRow::as_select())
        .load(conn)?;
    for old in legacy {
        if only.is_some_and(|id| id != old.repo_id) {
            continue;
        }
        if store::managed(conn, old.repo_id)? {
            continue;
        }
        let secret = mirror::cred::seal(app, Uuid::new_v4().to_string().as_bytes())?;
        conn.transaction::<(),Error,_>(|conn|{
            diesel::sql_query("INSERT INTO sync_connections(id,repo_id,provider,host,project,paused,webhook_secret) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
                .bind::<SqlUuid,_>(Uuid::now_v7()).bind::<SqlUuid,_>(old.repo_id).bind::<Text,_>(&old.provider).bind::<Text,_>(&old.host).bind::<Text,_>(&old.github_repo).bind::<Bool,_>(old.paused).bind::<diesel::sql_types::Binary,_>(&secret).execute(conn)?;
            let (org,repo)=local::load_repo(conn,old.repo_id)?;
            local::emit(conn,&org,&repo,"sync_connected","legacy",None)?;
            Ok(())
        })?;
    }
    Ok(())
}

fn key_for(
    c: &store::Connection,
    r: &remote::RemoteItem,
    replicas: &[store::Replica],
    known: &BTreeMap<String, Item>,
    legacy: &BTreeMap<String, String>,
) -> String {
    if matches!(
        r.item.kind.as_str(),
        "ref" | "settings" | "status" | "history"
    ) {
        return r.id.clone();
    }
    if let Some(mapped) = replicas
        .iter()
        .find(|p| p.remote_id.as_deref() == Some(&r.id))
    {
        return mapped.object_key.clone();
    }
    if let Some(key) = legacy.get(&r.id) {
        return key.clone();
    }
    // Markers recover a POST whose response was lost. They must name an
    // object in this repository; unknown markers never select another row.
    if let Some(key) = r.key.as_ref().filter(|key| {
        known.get(*key).is_some_and(|original| {
            original.kind == r.item.kind
                || (matches!(original.kind.as_str(), "pr" | "history") && r.item.kind == "summary")
                || (original.kind == "review" && r.item.kind == "comment")
        })
    }) {
        return key.clone();
    }
    format!("{}:{}", r.item.kind, Uuid::new_v5(&c.id, r.id.as_bytes()))
}
fn legacy_keys(
    conn: &mut PgConnection,
    c: &store::Connection,
) -> Result<BTreeMap<String, String>, Error> {
    let mut keys = BTreeMap::new();
    if c.provider != "github" {
        return Ok(keys);
    }
    if !mirror::state_for(conn, c.repo_id)?
        .is_some_and(|s| s.provider == c.provider && s.host == c.host && s.github_repo == c.project)
    {
        return Ok(keys);
    }
    use vcsstar_db::schema::{pr_comments as pc, pr_reviews as rv, pull_requests as pr};
    let prs: Vec<(Uuid, Option<i32>)> = pr::table
        .filter(pr::repo_id.eq(c.repo_id))
        .select((pr::id, pr::github_number))
        .load(conn)?;
    for (id, number) in prs {
        let Some(number) = number else {
            continue;
        };
        keys.insert(format!("pr:{number}"), format!("pr:{id}"));
        for (id, remote, kind) in pc::table
            .filter(pc::pr_id.eq(id))
            .select((pc::id, pc::github_id, pc::github_kind))
            .load::<(Uuid, Option<i64>, Option<String>)>(conn)?
        {
            if let Some(remote) = remote {
                keys.insert(
                    format!(
                        "{}:{number}:{remote}",
                        if kind.as_deref() == Some("review") {
                            "inline"
                        } else {
                            "comment"
                        }
                    ),
                    format!("comment:{id}"),
                );
            }
        }
        for (id, remote) in rv::table
            .filter(rv::pr_id.eq(id))
            .select((rv::id, rv::github_id))
            .load::<(Uuid, Option<i64>)>(conn)?
        {
            if let Some(remote) = remote {
                keys.insert(format!("review:{number}:{remote}"), format!("review:{id}"));
            }
        }
    }
    Ok(keys)
}
pub fn read_remote(
    app: &App,
    conn: &mut PgConnection,
    c: &store::Connection,
    client: &remote::Client,
    dir: &std::path::Path,
    known: &BTreeMap<String, Item>,
    replicas: &[store::Replica],
) -> Result<BTreeMap<String, remote::RemoteItem>, Error> {
    let raw = remote::snapshot(app, conn, c, client, dir)?;
    let legacy = legacy_keys(conn, c)?;
    let keys: BTreeMap<_, _> = raw
        .iter()
        .map(|r| (r.id.clone(), key_for(c, r, replicas, known, &legacy)))
        .collect();
    let mut out = BTreeMap::new();
    for mut r in raw {
        store::archive(conn, c.id, &r.id, &r.raw)?;
        if let Some(parent) = r.item.data.get("parent").and_then(Value::as_str) {
            let Some(key) = keys
                .get(parent)
                .or_else(|| known.get_key_value(parent).map(|(key, _)| key))
            else {
                return Err(Error::invalid(
                    "remote comment parent missing from complete snapshot",
                ));
            };
            r.item.data["parent"] = json!(key);
        }
        let key = keys[&r.id].clone();
        match out.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(r);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let previous: &mut remote::RemoteItem = entry.get_mut();
                previous.item.kind = "collision".into();
                previous.item.data = json!({"error":"multiple remote records claim this local object; repair their markers at the destination","first":previous.id,"other":r.id});
            }
        }
    }
    Ok(out)
}

pub fn run(app: &App, task: &Task) -> Result<bool, Error> {
    let mut conn = app.conn()?;
    let c = match store::connection(&mut conn, task.connection_id) {
        Ok(c) => c,
        Err(e) if e.code == Code::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    if let Some(body) = &task.webhook {
        let value: Value =
            serde_json::from_slice(body).map_err(|_| Error::invalid("webhook JSON"))?;
        store::archive(&mut conn, c.id, "webhook", &value)?;
    }
    if c.paused || (task.webhook.is_none() && c.poll_after > chrono::Utc::now()) {
        return Ok(true);
    }
    // One connection pass per repository. A crashed connection releases
    // this session lock; there is no clock-based claim to outlive.
    #[derive(QueryableByName)]
    struct Locked {
        #[diesel(sql_type=Bool)]
        locked: bool,
    }
    let bits = c.repo_id.as_u128();
    let lock_key = (bits as u64 ^ (bits >> 64) as u64) as i64;
    let locked: Locked = diesel::sql_query("SELECT pg_try_advisory_lock($1) AS locked")
        .bind::<BigInt, _>(lock_key)
        .get_result(&mut conn)?;
    if !locked.locked {
        return Ok(false);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_locked(app, &mut conn, &c, task)
    }));
    // Always unlock before returning the pooled connection.
    let unlock = diesel::sql_query("SELECT pg_advisory_unlock($1)")
        .bind::<BigInt, _>(lock_key)
        .execute(&mut conn);
    unlock?;
    let result = match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    match result {
        Ok(()) => Ok(true),
        Err(e) => {
            diesel::sql_query("UPDATE sync_connections SET last_error=$2,poll_after=now()+interval '60 seconds' WHERE id=$1").bind::<SqlUuid,_>(c.id).bind::<Text,_>(&e.message).execute(&mut conn)?;
            Err(e)
        }
    }
}
fn lock_native(conn: &mut PgConnection, repo: Uuid, key: &str) -> Result<(), Error> {
    // Lock the actual row to serialize with normal API mutations.
    let (kind, id) = key.split_once(':').unwrap_or((key, ""));
    if let Ok(id) = Uuid::parse_str(id) {
        let table = match kind {
            "pr" => "pull_requests",
            "comment" => "pr_comments",
            "review" => "pr_reviews",
            _ => return Ok(()),
        };
        diesel::sql_query(format!("SELECT id FROM {table} WHERE id=$1 FOR UPDATE"))
            .bind::<SqlUuid, _>(id)
            .execute(conn)?;
    } else if kind == "settings" {
        diesel::sql_query("SELECT id FROM repos WHERE id=$1 FOR UPDATE")
            .bind::<SqlUuid, _>(repo)
            .execute(conn)?;
    }
    Ok(())
}
fn fetch_item(
    app: &App,
    conn: &mut PgConnection,
    c: &store::Connection,
    _client: &remote::Client,
    dir: &std::path::Path,
    key: &str,
    r: &remote::RemoteItem,
) -> Result<(), Error> {
    let (source, sha) = if r.item.kind == "ref" {
        (
            key.trim_start_matches("ref:").to_string(),
            text(&r.item.data, "sha"),
        )
    } else if r.item.kind == "pr" && !text(&r.item.data, "head_sha").is_empty() {
        let number = r.id.split(':').nth(1).unwrap_or("");
        (
            if c.provider == "github" {
                format!("refs/pull/{number}/head")
            } else {
                format!("refs/merge-requests/{number}/head")
            },
            text(&r.item.data, "head_sha"),
        )
    } else {
        return Ok(());
    };
    if sha.is_empty() || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::invalid("remote revision is not a hex id"));
    }
    let client = remote::client_for(app, conn, c, None, "import_fetch")?;
    let destination = format!("refs/vcsstar/sync/{}/{}", c.id, super::digest(&json!(key)));
    match mirror::fetch_ref_for(
        dir,
        &c.git_url(app),
        (!client.token.is_empty()).then_some(client.token.as_str()),
        &source,
        &destination,
        c.provider == "github",
    ) {
        Ok(()) => {
            vcs::capture(
                app,
                conn,
                c.repo_id,
                dir,
                &[],
                &[vcsstar_vcs::RevId(sha.into())],
            )?;
            Ok(())
        }
        Err(e)
            if r.item.kind == "pr"
                && text(&r.item.data, "state") != "open"
                && e.message().contains("couldn't find remote ref") =>
        {
            // Some providers remove the hidden ref after a request closes.
            // Keep its record even when its object is no longer advertised.
            if app
                .store
                .backend()
                .object_exists(dir, &vcsstar_vcs::RevId(sha.into()), &vec![])
            {
                vcs::capture(
                    app,
                    conn,
                    c.repo_id,
                    dir,
                    &[],
                    &[vcsstar_vcs::RevId(sha.into())],
                )?;
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
fn run_locked(
    app: &App,
    conn: &mut vcsstar_db::DbConn,
    c: &store::Connection,
    task: &Task,
) -> Result<(), Error> {
    if let Some(body) = &task.webhook {
        let value: Value =
            serde_json::from_slice(body).map_err(|_| Error::invalid("webhook JSON"))?;
        store::archive(conn, c.id, "webhook", &value)?;
    }
    let client = remote::client(app, conn, c, None)?;
    if !c.setup_done {
        remote::setup(app, conn, c, &client)?;
    }
    let (_, repo) = local::load_repo(conn, c.repo_id)?;
    let dir = vcs::hydrate(app, conn, &repo)?;
    let mut known = local::snapshot(conn, &repo)?;
    let mut replicas = store::replicas(conn, c.id)?;
    let mut marker_objects = known.clone();
    for saved in store::objects(conn, c.repo_id)? {
        marker_objects
            .entry(saved.object_key)
            .or_insert(serde_json::from_value(saved.payload).map_err(Error::internal)?);
    }
    let mut remote = read_remote(app, conn, c, &client, &dir, &marker_objects, &replicas)?;
    // Import refs before requests and requests before their comments/reviews.
    let keys: BTreeSet<_> = known
        .keys()
        .chain(remote.keys())
        .chain(replicas.iter().map(|r| &r.object_key))
        .cloned()
        .collect();
    let mut keys: Vec<_> = keys.into_iter().collect();
    keys.sort_by_key(|k| {
        (
            if k.starts_with("ref:") {
                0
            } else if k == "settings" {
                1
            } else if k.starts_with("pr:") {
                2
            } else {
                3
            },
            k.clone(),
        )
    });
    let mut problems = Vec::new();
    for key in keys {
        // Reload local rows: another API request may have changed them while
        // the remote was read, or the preceding ref import changed a PR.
        let (org, repo) = local::load_repo(conn, c.repo_id)?;
        known = local::snapshot(conn, &repo)?;
        let local = known.get(&key).cloned();
        let r = remote.get(&key).cloned();
        if !replicas.iter().any(|p| p.object_key == key) {
            if let (Some(local), Some(r)) = (&local, &r) {
                if r.key.as_deref() == Some(key.as_str())
                    && remote::expand_projection(local, &r.item)
                        .is_some_and(|expanded| expanded.equivalent(local))
                {
                    store::save_replica(
                        conn,
                        c.id,
                        &key,
                        Some(&r.id),
                        Some(local),
                        Some(&r.item),
                        "synced",
                        None,
                    )?;
                    replicas = store::replicas(conn, c.id)?;
                }
            }
        }
        let replica = replicas.iter().find(|p| p.object_key == key).cloned();
        let lb = replica
            .as_ref()
            .map(|p| store::item(&p.local_base))
            .transpose()?
            .flatten();
        let rb = replica
            .as_ref()
            .map(|p| store::item(&p.remote_base))
            .transpose()?
            .flatten();
        if r.as_ref().is_some_and(|r| r.item.kind == "collision") {
            store::record_conflict(conn, c, &key, local.as_ref(), r.as_ref().map(|r| &r.item))?;
            problems.push(format!(
                "{key}: duplicate remote markers; repair the affected records"
            ));
            continue;
        }
        let mut decision = decide(
            local.as_ref(),
            r.as_ref().map(|r| &r.item),
            lb.as_ref(),
            rb.as_ref(),
        );
        let existing = store::conflicts(conn, c.repo_id)?;
        let preferred = store::policy(conn, c.repo_id)?.and_then(|p| p.preferred_source);
        if existing
            .iter()
            .any(|f| f.object_key == key && f.connection_id != c.id)
            && preferred.as_deref() != Some(c.id.to_string().as_str())
        {
            continue;
        }
        let own_conflict = existing
            .iter()
            .find(|f| f.object_key == key && f.connection_id == c.id);
        let preferred = store::policy(conn, c.repo_id)?.and_then(|p| p.preferred_source);
        if let Some(conflict) = own_conflict.filter(|f| f.resolution.is_some()) {
            let unchanged = store::item(&conflict.local_value)?
                .as_ref()
                .map(|i| &i.data)
                == local.as_ref().map(|i| &i.data)
                && store::item(&conflict.remote_value)?
                    .as_ref()
                    .map(|i| &i.data)
                    == r.as_ref().map(|r| &r.item.data);
            if unchanged {
                decision = if conflict.resolution.as_deref() == Some("remote") {
                    Decision::Import
                } else {
                    Decision::Export
                };
            } else {
                diesel::sql_query(
                    "UPDATE sync_conflicts SET resolution=NULL,resolved_by=NULL WHERE id=$1",
                )
                .bind::<SqlUuid, _>(conflict.id)
                .execute(conn)?;
                decision = Decision::Conflict;
            }
        } else if decision == Decision::Conflict || own_conflict.is_some() {
            decision = match preferred.as_deref() {
                Some("local") => Decision::Export,
                Some(id) if id == c.id.to_string() => Decision::Import,
                Some(id)
                    if store::objects(conn, c.repo_id)?
                        .iter()
                        .any(|o| o.object_key == key && o.source == id) =>
                {
                    Decision::Export
                }
                _ => Decision::Conflict,
            };
        }
        if decision == Decision::Conflict {
            conn.transaction::<(), Error, _>(|conn| {
                store::save_replica(
                    conn,
                    c.id,
                    &key,
                    r.as_ref().map(|r| r.id.as_str()),
                    lb.as_ref(),
                    rb.as_ref(),
                    "conflict",
                    None,
                )?;
                store::record_conflict(conn, c, &key, local.as_ref(), r.as_ref().map(|r| &r.item))?;
                if own_conflict.is_none() {
                    local::emit(conn, &org, &repo, "sync_conflict", &key, None)?;
                }
                Ok(())
            })?;
            continue;
        }
        let result = (|| -> Result<(), Error> {
            match decision {
                Decision::Equal => {
                    // Preserve different acknowledged projections (summaries).
                    if replica.is_none()
                        || local
                            .as_ref()
                            .zip(r.as_ref())
                            .is_some_and(|(l, r)| l.equivalent(&r.item))
                    {
                        store::save_replica(
                            conn,
                            c.id,
                            &key,
                            r.as_ref().map(|r| r.id.as_str()),
                            local.as_ref(),
                            r.as_ref().map(|r| &r.item),
                            "synced",
                            None,
                        )?;
                    }
                }
                Decision::Import => {
                    if let Some(r) = &r {
                        fetch_item(app, conn, c, &client, &dir, &key, r)?;
                    }
                    conn.transaction::<(),Error,_>(|conn|{
                        lock_native(conn,repo.id,&key)?;
                        let (_,latest_repo)=local::load_repo(conn,repo.id)?;
                        let latest=local::snapshot(conn,&latest_repo)?.remove(&key);
                        if latest.as_ref().map(|i|&i.data)!=local.as_ref().map(|i|&i.data){return Err(Error::new(Code::Conflict,"local item changed during sync"));}
                        if local.as_ref().is_some_and(|i|i.kind=="history") && r.as_ref().is_some_and(|r|r.item.kind=="summary") {
                            return Err(Error::new(Code::Conflict,"status history is an audit record; change the status at its source and keep the local history record"));
                        }
                        let expanded = match (&local,&r) {
                            (Some(original),Some(r)) => Some(remote::import_projection(original,&r.item,rb.as_ref()).ok_or_else(||Error::new(Code::Conflict,"the remote summary no longer contains a complete record; keep or edit the local record"))?),
                            _=>r.as_ref().map(|r|r.item.clone()),
                        };
                        let incoming=expanded.as_ref();
                        local::apply(app,conn,&org,&latest_repo,&key,incoming,&c.id.to_string())?;
                        store::save_replica(conn,c.id,&key,r.as_ref().map(|r|r.id.as_str()),incoming,r.as_ref().map(|r| &r.item),"synced",None)?;
                        Ok(())
                    })?;
                }
                Decision::Export => {
                    let parent = local
                        .as_ref()
                        .and_then(|i| i.data.get("parent"))
                        .and_then(Value::as_str);
                    let parent_remote = parent.and_then(|key| {
                        remote.get(key).map(|r| r.id.clone()).or_else(|| {
                            replicas
                                .iter()
                                .find(|p| p.object_key == key)
                                .and_then(|p| p.remote_id.clone())
                        })
                    });
                    if parent.is_some_and(|parent| {
                        replicas.iter().any(|p| {
                            p.object_key == parent
                                && p.outcome == "skipped"
                                && p.remote_id.is_none()
                        })
                    }) {
                        store::save_replica(
                            conn,
                            c.id,
                            &key,
                            None,
                            local.as_ref(),
                            None,
                            "skipped",
                            None,
                        )?;
                        store::save_object(conn, c.repo_id, &key, local.as_ref(), "local")?;
                        return Ok(());
                    }
                    let actor = local.as_ref().and_then(|i| i.actor);
                    let purpose = if key.starts_with("ref:") {
                        "mirror_git_push"
                    } else {
                        "mirror_api"
                    };
                    let mut acting = remote::client_for(app, conn, c, actor, purpose)?;
                    let mut written = remote::write(
                        app,
                        c,
                        &acting,
                        &dir,
                        &key,
                        local.as_ref(),
                        r.as_ref(),
                        parent_remote.as_deref(),
                    );
                    if acting.user
                        && written.as_ref().is_err_and(|e| {
                            e.code == Code::Forbidden
                                || e.code == Code::Unauthenticated
                                || (key.starts_with("ref:") && e.code == Code::PreconditionFailed)
                        })
                    {
                        if let Some(actor) = actor.filter(|_| {
                            written
                                .as_ref()
                                .is_err_and(|e| e.code == Code::Unauthenticated)
                        }) {
                            if c.provider == "github" {
                                crate::github_link::mark_broken(
                                    conn,
                                    actor,
                                    "destination refused the linked token; reconnect this account",
                                );
                            }
                            diesel::sql_query("UPDATE sync_links SET broken_reason=$4 WHERE identity_id=$1 AND provider=$2 AND host=$3").bind::<SqlUuid,_>(actor).bind::<Text,_>(&c.provider).bind::<Text,_>(&c.host).bind::<Text,_>("destination refused the linked token; reconnect this account").execute(conn)?;
                        }
                        acting = remote::client_for(app, conn, c, None, purpose)?;
                        written = remote::write(
                            app,
                            c,
                            &acting,
                            &dir,
                            &key,
                            local.as_ref(),
                            r.as_ref(),
                            parent_remote.as_deref(),
                        );
                    }
                    let written = written?;
                    store::save_replica(
                        conn,
                        c.id,
                        &key,
                        written.id.as_deref(),
                        local.as_ref(),
                        written.remote.as_ref(),
                        if written.skipped { "skipped" } else { "synced" },
                        None,
                    )?;
                    if let (Some(id), Some(item)) = (written.id, written.remote) {
                        remote.insert(
                            key.clone(),
                            remote::RemoteItem {
                                id,
                                key: Some(key.clone()),
                                item,
                                raw: Value::Null,
                            },
                        );
                    }
                    replicas = store::replicas(conn, c.id)?;
                }
                Decision::Conflict => unreachable!(),
            }
            if let Some(conflict) = own_conflict {
                conn.transaction::<(), Error, _>(|conn| {
                    diesel::sql_query("UPDATE sync_conflicts SET resolved_at=now() WHERE id=$1")
                        .bind::<SqlUuid, _>(conflict.id)
                        .execute(conn)?;
                    local::emit(conn, &org, &repo, "sync_resolved", &key, None)
                })?;
            }
            // Keep complete native records, including those exported as a summary.
            let (_, repo) = local::load_repo(conn, c.repo_id)?;
            let fresh = local::snapshot(conn, &repo)?.remove(&key);
            let source = if decision == Decision::Import
                || preferred.as_deref() == Some(c.id.to_string().as_str())
            {
                c.id.to_string()
            } else {
                store::objects(conn, c.repo_id)?
                    .into_iter()
                    .find(|o| {
                        o.object_key == key
                            && serde_json::from_value::<Item>(o.payload.clone())
                                .is_ok_and(|old| fresh.as_ref().is_some_and(|f| old.equivalent(f)))
                    })
                    .map(|o| o.source)
                    .unwrap_or_else(|| "local".into())
            };
            store::save_object(conn, c.repo_id, &key, fresh.as_ref(), &source)?;
            Ok(())
        })();
        if let Err(error) = result {
            if error.code == Code::Conflict {
                store::save_replica(
                    conn,
                    c.id,
                    &key,
                    r.as_ref().map(|r| r.id.as_str()),
                    lb.as_ref(),
                    rb.as_ref(),
                    "conflict",
                    Some(&error.message),
                )?;
                store::record_conflict(conn, c, &key, local.as_ref(), r.as_ref().map(|r| &r.item))?;
            } else {
                store::save_replica(
                    conn,
                    c.id,
                    &key,
                    r.as_ref()
                        .map(|r| r.id.as_str())
                        .or_else(|| replica.as_ref().and_then(|p| p.remote_id.as_deref())),
                    lb.as_ref(),
                    rb.as_ref(),
                    "blocked",
                    Some(&error.message),
                )?;
            }
            problems.push(format!("{key}: {}", error.message));
        }
    }
    let error = (!problems.is_empty()).then(|| problems.join("; "));
    diesel::sql_query("UPDATE sync_connections SET completed=GREATEST(completed,$2),poll_after=now()+interval '60 seconds',last_success_at=CASE WHEN $3::text IS NULL THEN now() ELSE last_success_at END,last_error=$3 WHERE id=$1")
        .bind::<SqlUuid,_>(c.id).bind::<BigInt,_>(c.requested).bind::<diesel::sql_types::Nullable<Text>,_>(error).execute(conn)?;
    Ok(())
}

/// Authenticate before capturing the raw webhook in the durable queue.
pub fn accept_webhook(
    app: &App,
    id: Uuid,
    signature: Option<&str>,
    gitlab_token: Option<&str>,
    delivery: Option<&str>,
    body: &[u8],
) -> Result<(), Error> {
    use hmac::{Hmac, Mac};
    let mut conn = app.conn()?;
    let c = store::connection(&mut conn, id)?;
    let secret = mirror::cred::open(app, &c.webhook_secret)?;
    let valid = if c.provider == "github" {
        signature
            .is_some_and(|s| crate::github::verify_webhook_signature(secret.as_bytes(), body, s))
    } else if let Some(supplied) = gitlab_token {
        let mut actual =
            Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).map_err(Error::internal)?;
        actual.update(b"gitlab-webhook-token");
        let mut candidate =
            Hmac::<sha2::Sha256>::new_from_slice(supplied.as_bytes()).map_err(Error::internal)?;
        candidate.update(b"gitlab-webhook-token");
        actual
            .verify_slice(&candidate.finalize().into_bytes())
            .is_ok()
    } else {
        false
    };
    if !valid {
        return Err(Error::forbidden("invalid webhook authentication"));
    }
    let value: Value =
        serde_json::from_slice(body).map_err(|_| Error::invalid("webhook body is not JSON"))?;
    let project = if c.provider == "github" {
        value.pointer("/repository/full_name")
    } else {
        value.pointer("/project/path_with_namespace")
    }
    .and_then(Value::as_str);
    if project.is_some_and(|p| p != c.project) {
        return Err(Error::invalid(
            "webhook project does not match this connection",
        ));
    }
    let mut payload = vec![];
    ciborium::ser::into_writer(
        &Task {
            connection_id: id,
            webhook: Some(body.to_vec()),
        },
        &mut payload,
    )
    .map_err(Error::internal)?;
    let digest = super::digest(&value);
    let key = format!("sync-hook:{id}:{}:{digest}", delivery.unwrap_or("delivery"));
    match app.submit_task(QUEUE, &key, payload, 2) {
        Err(e) if e.code == Code::AlreadyExists => Ok(()),
        other => other,
    }
}
