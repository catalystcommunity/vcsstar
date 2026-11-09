//! Transactional outbox writer. Call inside the same transaction as the
//! state change the event describes.

use diesel::prelude::*;
use vscstar_db::models::NewEvent;
use vscstar_db::schema::events;
use vscstar_types::{encode_event, Event, EventPayload, Identity, RepoRef};

use crate::{now_ms, Error};

pub struct Emit<'a> {
    pub lane: String,
    pub kind: &'a str,
    pub repo_id: Option<uuid::Uuid>,
    pub org_id: Option<uuid::Uuid>,
    pub actor_id: Option<uuid::Uuid>,
    pub actor: Option<Identity>,
    pub repo: Option<RepoRef>,
    pub org: Option<String>,
    pub subject: Option<String>,
    pub payload: EventPayload,
}

pub fn lane_for_repo(org: &str, repo: &str) -> String {
    format!("{org}/{repo}")
}

pub fn lane_for_org(org: &str) -> String {
    org.to_string()
}

/// Append one event. Returns the per-lane sequence.
pub fn emit(conn: &mut PgConnection, e: Emit<'_>) -> Result<i64, Error> {
    let seq = vscstar_db::next_lane_seq(conn, &e.lane)?;
    let event = Event {
        lane: e.lane.clone(),
        seq: seq as u64,
        kind: e.kind.to_string(),
        actor: e.actor,
        repo: e.repo,
        org: e.org,
        subject: e.subject.clone(),
        at: now_ms(),
        payload: e.payload,
    };
    let row = NewEvent {
        lane: &e.lane,
        seq,
        repo_id: e.repo_id,
        org_id: e.org_id,
        kind: e.kind,
        actor: e.actor_id,
        subject: e.subject.as_deref(),
        payload_cbor: encode_event(&event),
    };
    diesel::insert_into(events::table)
        .values(&row)
        .execute(conn)?;
    Ok(seq)
}
