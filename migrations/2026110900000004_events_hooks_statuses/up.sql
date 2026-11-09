-- Phase 2: outbox dispatch marker, outbound hooks, commit statuses.

-- The dispatcher fans each event out once. Rows with dispatched_at IS NULL
-- are the backlog; the partial index keeps the scan cheap at any size.
ALTER TABLE events ADD COLUMN dispatched_at timestamptz;
CREATE INDEX events_undispatched_idx ON events (created_at) WHERE dispatched_at IS NULL;

CREATE TABLE hook_endpoints (
    id              uuid PRIMARY KEY,
    org_id          uuid NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    repo_id         uuid REFERENCES repos (id) ON DELETE CASCADE,
    kind            text NOT NULL CHECK (kind IN ('https', 'corndogs')),
    url             text,
    corndogs_queue  text,
    -- HMAC-SHA256 key for https endpoints. Stored as given; a shared secret
    -- the receiver also holds, not a credential of ours.
    secret          bytea,
    event_filter    text[] NOT NULL DEFAULT '{}',
    content_type    text NOT NULL DEFAULT 'json' CHECK (content_type IN ('json', 'cbor')),
    active          boolean NOT NULL DEFAULT true,
    unordered       boolean NOT NULL DEFAULT false,
    created_by      uuid REFERENCES identities (id) ON DELETE SET NULL,
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX hook_endpoints_org_idx ON hook_endpoints (org_id, repo_id);

CREATE TABLE hook_deliveries (
    id              uuid PRIMARY KEY,
    endpoint_id     uuid NOT NULL REFERENCES hook_endpoints (id) ON DELETE CASCADE,
    event_lane      text NOT NULL,
    event_seq       bigint NOT NULL,
    event_kind      text NOT NULL,
    state           text NOT NULL CHECK (state IN ('pending', 'delivered', 'dead')),
    attempts        integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    last_status     integer,
    last_error      text,
    delivered_at    timestamptz,
    -- The event payload travels with the delivery so redelivery survives
    -- outbox archival.
    payload_cbor    bytea NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX hook_deliveries_endpoint_idx ON hook_deliveries (endpoint_id, state, created_at);

CREATE TABLE statuses (
    id              uuid PRIMARY KEY,
    repo_id         uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    sha             text NOT NULL,
    context         text NOT NULL,
    state           text NOT NULL CHECK (state IN ('pending', 'success', 'failure', 'error')),
    target_url      text,
    description     text,
    reporter        uuid REFERENCES identities (id) ON DELETE SET NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repo_id, sha, context)
);
CREATE INDEX statuses_repo_sha_idx ON statuses (repo_id, sha);
