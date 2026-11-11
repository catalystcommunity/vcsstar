-- Rows are kept forever (decision 2026-11-11), so the month partitioning
-- of the outbox buys nothing and costs a partition per month plus an index
-- probe per partition on every (lane, seq) poll. One plain table instead.
CREATE TABLE events_plain (
    lane            text NOT NULL,
    seq             bigint NOT NULL,
    repo_id         uuid,
    org_id          uuid,
    kind            text NOT NULL,
    actor           uuid,
    subject         text,
    payload_cbor    bytea NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    origin          text,
    PRIMARY KEY (lane, seq)
);
INSERT INTO events_plain (lane, seq, repo_id, org_id, kind, actor, subject, payload_cbor, created_at, origin)
    SELECT lane, seq, repo_id, org_id, kind, actor, subject, payload_cbor, created_at, origin FROM events;
ALTER TABLE events_plain ADD COLUMN dispatched_at timestamptz;
UPDATE events_plain p SET dispatched_at = e.dispatched_at FROM events e WHERE e.lane = p.lane AND e.seq = p.seq;
DROP TABLE events;
ALTER TABLE events_plain RENAME TO events;
CREATE INDEX events_repo_created_idx ON events (repo_id, created_at);
CREATE INDEX events_kind_created_idx ON events (kind, created_at);
CREATE INDEX events_undispatched_idx ON events (created_at) WHERE dispatched_at IS NULL;

-- Repack worker: when a repository's history was last consolidated, and
-- whether a segment carries a reachability bitmap beside its index.
ALTER TABLE repos ADD COLUMN repacked_at timestamptz;
ALTER TABLE repo_segments ADD COLUMN has_bitmap boolean NOT NULL DEFAULT false;
