-- State that every node must share so API and worker pods scale out:
-- the GitHub lane breakers, the per-token rate limit table, and the
-- pull-sync clock. Read and written by `vcsstar_core::github_health`.
CREATE TABLE github_lanes (
    lane text PRIMARY KEY,
    consecutive_failures integer NOT NULL DEFAULT 0,
    opens integer NOT NULL DEFAULT 0,
    open_until timestamptz,
    probing_since timestamptz,
    last_error text,
    last_failure_at timestamptz,
    last_success_at timestamptz,
    recovered_at timestamptz
);
INSERT INTO github_lanes (lane) VALUES ('api'), ('git');

CREATE TABLE github_rate_limits (
    token_label text PRIMARY KEY,
    remaining bigint NOT NULL,
    "limit" bigint NOT NULL,
    reset_at timestamptz NOT NULL,
    hold_until timestamptz,
    seen_at timestamptz NOT NULL DEFAULT now()
);

-- When a worker last claimed the pull-based sync pass of the repository
-- (the watermark of what it processed is `last_pull_sync_at`).
ALTER TABLE mirror_state ADD COLUMN last_pull_pass_at timestamptz;

-- GitHub's `updated_at` of the object as last applied here, so a stale
-- (out-of-order or redelivered) edit never overwrites a newer one.
ALTER TABLE pr_comments ADD COLUMN github_updated_at timestamptz;
ALTER TABLE pull_requests ADD COLUMN github_updated_at timestamptz;

-- Two dispatchers may expand the same event; the delivery row is unique
-- per endpoint and event.
DELETE FROM hook_deliveries a USING hook_deliveries b
    WHERE a.id > b.id AND a.endpoint_id = b.endpoint_id
      AND a.event_lane = b.event_lane AND a.event_seq = b.event_seq;
CREATE UNIQUE INDEX hook_deliveries_event_uidx
    ON hook_deliveries (endpoint_id, event_lane, event_seq);
