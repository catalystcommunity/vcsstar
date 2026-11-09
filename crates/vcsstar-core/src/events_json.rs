//! JSON projection of outbox events for HTTPS webhooks and the UI feed.
//! Field names follow the contract; the payload carries a `type` tag.

use serde_json::{json, Value as Json};
use vcsstar_types::{Event, EventPayload};

pub fn event_to_json(e: &Event) -> Json {
    let payload = match &e.payload {
        EventPayload::Variant0(p) => json!({
            "type": "push", "ref": p.r#ref, "before": p.before, "after": p.after,
            "forced": p.forced,
            "commits": p.commits.iter().map(|c| json!({
                "id": c.id, "author_name": c.author_name, "author_email": c.author_email,
                "message_head": c.message_head, "signature": c.signature,
            })).collect::<Vec<_>>(),
        }),
        EventPayload::Variant1(p) => json!({
            "type": "ref", "ref": p.r#ref, "before": p.before, "after": p.after,
        }),
        EventPayload::Variant2(p) => json!({
            "type": "pull_request", "number": p.number, "head_sha": p.head_sha,
            "base_ref": p.base_ref, "state": p.state, "merge_kind": p.merge_kind,
        }),
        EventPayload::Variant3(p) => json!({
            "type": "release", "tag": p.tag, "name": p.name, "body": p.body,
            "draft": p.draft, "prerelease": p.prerelease,
        }),
        EventPayload::Variant4(p) => json!({
            "type": "mirror", "object_key": p.object_key, "error": p.error,
            "action_kind": p.action_kind,
        }),
        EventPayload::Variant5(p) => json!({ "type": "settings", "changed": p.changed }),
        EventPayload::Variant6(p) => json!({
            "type": "status", "sha": p.sha, "context": p.context, "state": p.state,
        }),
        EventPayload::Variant7(p) => json!({
            "type": "membership", "identity": p.identity, "role": p.role, "team": p.team,
        }),
    };
    json!({
        "lane": e.lane,
        "seq": e.seq,
        "kind": e.kind,
        "at": e.at,
        "actor": e.actor.as_ref().map(|a| json!({
            "user_id": a.user_id, "domain": a.domain, "handle": a.handle,
            "display_name": a.display_name,
        })),
        "repo": e.repo.as_ref().map(|r| json!({"org": r.org, "name": r.name, "instance": r.instance})),
        "org": e.org,
        "subject": e.subject,
        "payload": payload,
    })
}
