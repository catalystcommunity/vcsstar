-- Phase 3: GitHub mirror state, per-key actions, attempts, credentials.

CREATE TABLE credentials (
    id                  uuid PRIMARY KEY,
    org_id              uuid NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    repo_id             uuid REFERENCES repos (id) ON DELETE CASCADE,
    provider            text NOT NULL,
    purpose             text NOT NULL CHECK (purpose IN ('mirror_git_push', 'mirror_api', 'import_fetch', 'webhook_inbound')),
    auth_method         text NOT NULL CHECK (auth_method IN ('pat', 'github_app')),
    host                text NOT NULL,
    -- `env:<VAR>` reads the process environment (operator-managed Secret);
    -- `stored` means the value is in secret_ciphertext, sealed with the
    -- instance credential key.
    secret_ref          text NOT NULL,
    secret_ciphertext   bytea,
    enabled             boolean NOT NULL DEFAULT true,
    created_by          uuid REFERENCES identities (id) ON DELETE SET NULL,
    created_at          timestamptz NOT NULL DEFAULT now(),
    UNIQUE (org_id, repo_id, provider, purpose)
);

CREATE TABLE mirror_state (
    repo_id                 uuid PRIMARY KEY REFERENCES repos (id) ON DELETE CASCADE,
    github_repo             text NOT NULL,
    mode                    text NOT NULL DEFAULT 'authoritative_vcsstar'
                            CHECK (mode IN ('authoritative_vcsstar', 'import_drift', 'flag_drift')),
    paused                  boolean NOT NULL DEFAULT false,
    cursor_seq              bigint NOT NULL DEFAULT 0,
    last_success_at         timestamptz,
    last_error              text,
    last_full_reconcile_at  timestamptz,
    created_at              timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE mirror_refs (
    repo_id             uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    ref                 text NOT NULL,
    mirrored_sha        text NOT NULL,
    last_pushed_at      timestamptz,
    last_verified_at    timestamptz,
    PRIMARY KEY (repo_id, ref)
);

CREATE TABLE mirror_actions (
    id              uuid PRIMARY KEY,
    repo_id         uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    event_lane      text NOT NULL,
    event_seq       bigint NOT NULL,
    action_kind     text NOT NULL CHECK (action_kind IN ('git_push', 'api_call')),
    object_key      text NOT NULL,
    state           text NOT NULL CHECK (state IN ('pending', 'applied', 'blocked', 'skipped')),
    blocked_reason  text,
    blocked_at      timestamptz,
    credential_purpose text NOT NULL,
    payload_cbor    bytea NOT NULL,
    attempts        integer NOT NULL DEFAULT 0,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);
-- At most one pending action per object key: newer events refresh it.
CREATE UNIQUE INDEX mirror_actions_pending_key ON mirror_actions (repo_id, object_key) WHERE state = 'pending';
CREATE INDEX mirror_actions_repo_state_idx ON mirror_actions (repo_id, state, created_at);

CREATE TABLE mirror_attempts (
    id              uuid PRIMARY KEY,
    action_id       uuid NOT NULL REFERENCES mirror_actions (id) ON DELETE CASCADE,
    started_at      timestamptz NOT NULL DEFAULT now(),
    finished_at     timestamptz,
    ok              boolean NOT NULL DEFAULT false,
    http_status     integer,
    error           text
);
CREATE INDEX mirror_attempts_action_idx ON mirror_attempts (action_id, started_at);
