//! The ingest worker role: process raw inbound items that receivers captured
//! into corndogs (`ingest:github`). A GitHub push webhook is a hint to
//! verify one ref now; nothing in its payload is applied directly. Pull
//! request, review, comment, status, and check events are mirrored into
//! vcsstar rows by `github_sync` (the payload is the source there, as the
//! REST object it carries is what GitHub would answer).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use vcsstar_core::mirror::{self, ActionPayload, ActionSpec, EventRef};
use vcsstar_core::App;
use vcsstar_db::schema::{mirror_refs, mirror_state, orgs, repos};

use crate::queue::{Claimed, Queue};

pub const QUEUE: &str = "ingest:github";

/// What the receiver captured: the raw body plus the headers that matter.
#[derive(Debug, Serialize, Deserialize)]
pub struct GithubInbound {
    pub delivery_id: String,
    pub event: String,
    pub body: Vec<u8>,
    pub received_at_ms: i64,
}

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut queue: Option<Queue> = None;
    let queues = vec![QUEUE.to_string()];
    while !stop.load(Ordering::SeqCst) {
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => {
                    tracing::info!("ingest worker connected to corndogs");
                    queue = Some(q);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ingest worker cannot reach corndogs");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        let mut drop_connection = false;
        {
            let q = queue.as_ref().expect("connected");
            match q.claim(&queues, Duration::from_secs(120)) {
                Ok(Some(claimed)) => {
                    if let Err(e) = handle(&app, q, &claimed) {
                        tracing::warn!(error = %e, "ingest task failed");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "ingest claim failed; reconnecting");
                    drop_connection = true;
                }
            }
        }
        if drop_connection {
            queue = None;
        }
        crate::pause(&stop, Duration::from_secs(2));
    }
    Ok(())
}

fn handle(app: &App, q: &Queue, claimed: &Claimed) -> anyhow::Result<()> {
    let Some(item): Option<GithubInbound> = crate::decode(&claimed.payload) else {
        q.complete(claimed)?;
        return Ok(());
    };
    // Everything a delivery changes came from GitHub: stamp its events so
    // the dispatcher never mirrors them back.
    match vcsstar_core::outbox::with_origin("github", || process(app, &item)) {
        Ok(note) => {
            let outcome = if note.starts_with("ignored") || note.starts_with("no mirror") {
                "ignored"
            } else {
                "applied"
            };
            vcsstar_core::metrics::github_ingest(&item.event, outcome);
            tracing::info!(delivery = %item.delivery_id, event = %item.event, note, "github webhook processed");
            q.complete(claimed)?;
        }
        Err(e) => {
            vcsstar_core::metrics::github_ingest(&item.event, "retry");
            tracing::warn!(delivery = %item.delivery_id, event = %item.event, error = %e, "github webhook processing failed; retry");
            q.park(claimed, Duration::from_secs(60))?;
        }
    }
    Ok(())
}

/// Route one delivery: `push` is a targeted verification of the hinted
/// ref against the mirrored state; the pull request family is mirrored
/// into vcsstar rows.
pub fn process(app: &App, item: &GithubInbound) -> anyhow::Result<String> {
    let body: serde_json::Value = serde_json::from_slice(&item.body)?;
    match item.event.as_str() {
        "push" => process_push(app, &body),
        "pull_request"
        | "pull_request_review"
        | "pull_request_review_comment"
        | "issue_comment"
        | "status"
        | "check_run" => {
            let full_name = body
                .pointer("/repository/full_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("payload without repository.full_name"))?;
            let mut conn = app.conn()?;
            let Some((org, repo, state)) = mirror_target(&mut conn, full_name)? else {
                return Ok(format!("no mirror for {full_name}"));
            };
            vcsstar_core::github_sync::ingest(
                app,
                &mut conn,
                &org,
                &repo,
                &state,
                &item.event,
                &body,
            )
            .map_err(|e| anyhow::anyhow!("{}: {}", e.code as u32, e.message))
        }
        other => Ok(format!("ignored event {other}")),
    }
}

/// The mirrored repository a GitHub `owner/name` belongs to.
fn mirror_target(
    conn: &mut PgConnection,
    full_name: &str,
) -> anyhow::Result<
    Option<(
        vcsstar_db::models::Org,
        vcsstar_db::models::Repo,
        mirror::StateRow,
    )>,
> {
    use vcsstar_db::models::{Org, Repo};
    let found: Option<(Repo, Org, mirror::StateRow)> = mirror_state::table
        .inner_join(repos::table.inner_join(orgs::table))
        .filter(mirror_state::github_repo.eq(full_name))
        .select((
            Repo::as_select(),
            Org::as_select(),
            mirror::StateRow::as_select(),
        ))
        .first(conn)
        .optional()?;
    Ok(found.map(|(r, o, s)| (o, r, s)))
}

fn process_push(app: &App, body: &serde_json::Value) -> anyhow::Result<String> {
    let full_name = body
        .pointer("/repository/full_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("push payload without repository.full_name"))?;
    let r#ref = body
        .get("ref")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("push payload without ref"))?;
    let after = body.get("after").and_then(|v| v.as_str()).unwrap_or("");
    let deleted = body
        .get("deleted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut conn = app.conn()?;
    let Some((repo_id, org, repo, mode)): Option<(uuid::Uuid, String, String, String)> =
        mirror_state::table
            .inner_join(repos::table.inner_join(orgs::table))
            .filter(mirror_state::github_repo.eq(full_name))
            .select((repos::id, orgs::name, repos::name, mirror_state::mode))
            .first(&mut conn)
            .optional()?
    else {
        return Ok(format!("no mirror for {full_name}"));
    };
    let mirrored = mirror::mirrored_sha(&mut conn, repo_id, r#ref)?;
    let github_now = if deleted { None } else { Some(after) };
    if mirrored.as_deref() == github_now {
        diesel::update(
            mirror_refs::table
                .filter(mirror_refs::repo_id.eq(repo_id))
                .filter(mirror_refs::ref_.eq(r#ref)),
        )
        .set(mirror_refs::last_verified_at.eq(Utc::now()))
        .execute(&mut conn)?;
        return Ok(format!("{ref} verified"));
    }
    let _ = mode;
    let (org_row, repo_row) = {
        use vcsstar_db::models::{Org, Repo};
        let r: Repo = repos::table
            .find(repo_id)
            .select(Repo::as_select())
            .first(&mut conn)?;
        let o: Org = orgs::table
            .find(r.org_id)
            .select(Org::as_select())
            .first(&mut conn)?;
        (o, r)
    };
    let Some(state) = mirror::state_for(&mut conn, repo_id)? else {
        return Ok("mirror state vanished".into());
    };
    let dir = vcsstar_core::vcs::hydrate(app, &mut conn, &repo_row)?;
    let token = mirror::cred::token_for(app, &mut conn, org_row.id, repo_id, "mirror_git_push")
        .ok()
        .map(|(_, t)| t);
    let push = mirror::note_drift(
        app,
        &mut conn,
        &org_row,
        &repo_row,
        &state,
        &dir,
        token.as_deref(),
        r#ref,
        github_now,
    )?;
    if !push {
        return Ok(format!("{ref} drifted; handled by mode {}", state.mode));
    }
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
        lane: &format!("{org}/{repo}"),
        seq: 0,
        kind: "github_webhook",
        payload_cbor: &[],
    };
    let id = mirror::upsert_action(&mut conn, &ev, &spec)?;
    mirror::submit_action(app, &org, &repo, id, 0).map_err(|e| anyhow::anyhow!(e.message))?;
    Ok(format!("{ref} drifted; convergence scheduled"))
}
