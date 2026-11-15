-- Each repository can connect to several independently controlled remotes.
CREATE TABLE sync_connections (
    id uuid PRIMARY KEY,
    repo_id uuid NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('github', 'gitlab')),
    host text NOT NULL,
    project text NOT NULL,
    paused boolean NOT NULL DEFAULT false,
    create_remote boolean NOT NULL DEFAULT false,
    automatic_webhook boolean NOT NULL DEFAULT false,
    webhook_secret bytea NOT NULL,
    setup_done boolean NOT NULL DEFAULT false,
    requested bigint NOT NULL DEFAULT 1,
    completed bigint NOT NULL DEFAULT 0,
    poll_after timestamptz NOT NULL DEFAULT now(),
    last_success_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(repo_id, provider, host, project),
    UNIQUE(provider, host, project)
);
CREATE TABLE sync_policy (
    repo_id uuid PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    preferred_source text,
    resolver_identity uuid REFERENCES identities(id),
    resolver_team uuid REFERENCES teams(id),
    CHECK (preferred_source IS NOT NULL OR resolver_identity IS NOT NULL OR resolver_team IS NOT NULL)
);
CREATE TABLE sync_objects (
    repo_id uuid NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    object_key text NOT NULL,
    payload jsonb NOT NULL,
    revision bigint NOT NULL DEFAULT 1,
    source text NOT NULL DEFAULT 'local',
    deleted boolean NOT NULL DEFAULT false,
    PRIMARY KEY(repo_id, object_key)
);
CREATE TABLE sync_replicas (
    connection_id uuid NOT NULL REFERENCES sync_connections(id) ON DELETE CASCADE,
    object_key text NOT NULL,
    remote_id text,
    local_base jsonb,
    remote_base jsonb,
    outcome text NOT NULL DEFAULT 'pending',
    last_error text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(connection_id, object_key),
    UNIQUE(connection_id, remote_id)
);
CREATE TABLE sync_conflicts (
    id uuid PRIMARY KEY,
    repo_id uuid NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    object_key text NOT NULL,
    connection_id uuid NOT NULL REFERENCES sync_connections(id) ON DELETE CASCADE,
    local_value jsonb,
    remote_value jsonb,
    resolution text CHECK(resolution IN ('local', 'remote')),
    resolved_by uuid REFERENCES identities(id),
    resolved_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX sync_conflicts_open ON sync_conflicts(repo_id, object_key, connection_id)
    WHERE resolved_at IS NULL;
-- Keep original provider records even when a destination needs a summary.
CREATE TABLE sync_history (
    connection_id uuid NOT NULL REFERENCES sync_connections(id) ON DELETE CASCADE,
    remote_id text NOT NULL,
    digest text NOT NULL,
    payload jsonb NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(connection_id, remote_id, digest)
);
CREATE TABLE sync_links (
    identity_id uuid NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
    provider text NOT NULL CHECK(provider IN ('github', 'gitlab')),
    host text NOT NULL,
    remote_user_id text NOT NULL,
    login text NOT NULL,
    token bytea NOT NULL,
    broken_reason text,
    PRIMARY KEY(identity_id, provider, host),
    UNIQUE(provider, host, remote_user_id)
);
