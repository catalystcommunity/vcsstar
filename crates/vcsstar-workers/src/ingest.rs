//! The ingest worker role: process raw inbound items that receivers captured
//! into corndogs (`ingest:github`). A GitHub push webhook is a hint to
//! verify one ref now; nothing in its payload is applied directly.

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
    match process(app, &item) {
        Ok(note) => {
            tracing::info!(delivery = %item.delivery_id, event = %item.event, note, "github webhook processed");
            q.complete(claimed)?;
        }
        Err(e) => {
            tracing::warn!(delivery = %item.delivery_id, error = %e, "github webhook processing failed; retry");
            q.park(claimed, Duration::from_secs(60))?;
        }
    }
    Ok(())
}

/// Targeted verification of the hinted ref against the mirrored state.
pub fn process(app: &App, item: &GithubInbound) -> anyhow::Result<String> {
    if item.event != "push" {
        return Ok(format!("ignored event {}", item.event));
    }
    let body: serde_json::Value = serde_json::from_slice(&item.body)?;
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
