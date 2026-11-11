//! Transactional outbox writer. Call inside the same transaction as the
//! state change the event describes.

use diesel::prelude::*;
use vcsstar_db::models::NewEvent;
use vcsstar_db::schema::events;
use vcsstar_types::{encode_event, Event, EventPayload, Identity, RepoRef};

use crate::{now_ms, Error};

thread_local! {
    static ORIGIN: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with every event it emits on this thread stamped with
/// `origin`. The ingest worker wraps the application of one GitHub
/// delivery in `with_origin("github", ..)`: whatever rows and events that
/// produces came from GitHub, whoever the actor is, and the dispatcher
/// must not mirror them back.
pub fn with_origin<T>(origin: &'static str, f: impl FnOnce() -> T) -> T {
    let previous = ORIGIN.with(|o| o.replace(Some(origin)));
    struct Reset(Option<&'static str>);
    impl Drop for Reset {
        fn drop(&mut self) {
            let p = self.0;
            ORIGIN.with(|o| *o.borrow_mut() = p);
        }
    }
    let _reset = Reset(previous);
    f()
}

/// The origin events emitted on this thread carry right now.
pub fn current_origin() -> Option<&'static str> {
    ORIGIN.with(|o| *o.borrow())
}

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
    let seq = vcsstar_db::next_lane_seq(conn, &e.lane)?;
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
        origin: current_origin(),
    };
    diesel::insert_into(events::table)
        .values(&row)
        .execute(conn)?;
    Ok(seq)
}

#[cfg(test)]
mod tests {
    use super::{current_origin, with_origin};

    #[test]
    fn origin_is_scoped_to_the_closure() {
        assert_eq!(current_origin(), None);
        let inner = with_origin("github", || {
            assert_eq!(current_origin(), Some("github"));
            with_origin("peer", current_origin)
        });
        assert_eq!(inner, Some("peer"));
        assert_eq!(current_origin(), None);
    }
}
