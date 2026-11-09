-- Transactional outbox (per-lane sequence, month partitions) and audit log.

CREATE TABLE lane_counters (
    lane        text PRIMARY KEY,
    next_seq    bigint NOT NULL DEFAULT 1
);

CREATE TABLE events (
    lane            text NOT NULL,
    seq             bigint NOT NULL,
    repo_id         uuid,
    org_id          uuid,
    kind            text NOT NULL,
    actor           uuid,
    subject         text,
    payload_cbor    bytea NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (lane, seq, created_at)
) PARTITION BY RANGE (created_at);

-- A default partition catches rows before the maintenance job makes the
-- month partition. The job detaches and splits it when needed.
CREATE TABLE events_default PARTITION OF events DEFAULT;

CREATE INDEX events_repo_created_idx ON events (repo_id, created_at);
CREATE INDEX events_kind_created_idx ON events (kind, created_at);

-- next_lane_seq(lane) allocates the next per-lane sequence inside the
-- caller's transaction. The row lock orders writers on one lane.
CREATE FUNCTION next_lane_seq(p_lane text) RETURNS bigint
LANGUAGE plpgsql AS $$
DECLARE
    v bigint;
BEGIN
    INSERT INTO lane_counters (lane, next_seq) VALUES (p_lane, 2)
    ON CONFLICT (lane) DO UPDATE SET next_seq = lane_counters.next_seq + 1
    RETURNING next_seq - 1 INTO v;
    RETURN v;
END $$;

CREATE TABLE audit_log (
    id              uuid PRIMARY KEY,
    actor           uuid,
    acting_via      text,
    action          text NOT NULL,
    resource        text NOT NULL,
    detail_cbor     bytea,
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_log_created_idx ON audit_log (created_at);
CREATE INDEX audit_log_resource_idx ON audit_log (resource, created_at);
