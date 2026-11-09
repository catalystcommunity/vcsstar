-- Organizations, repositories, refs, segments, grants, protections.

CREATE TABLE orgs (
    id              uuid PRIMARY KEY,
    name            text NOT NULL UNIQUE,
    display_name    text,
    github_owner    text,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE org_members (
    org_id          uuid NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    role            text NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, identity_id)
);

CREATE TABLE teams (
    id              uuid PRIMARY KEY,
    org_id          uuid NOT NULL REFERENCES orgs (id) ON DELETE CASCADE,
    name            text NOT NULL,
    UNIQUE (org_id, name)
);

CREATE TABLE team_members (
    team_id         uuid NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    PRIMARY KEY (team_id, identity_id)
);

CREATE TABLE repos (
    id                  uuid PRIMARY KEY,
    org_id              uuid NOT NULL REFERENCES orgs (id) ON DELETE RESTRICT,
    name                text NOT NULL,
    visibility          text NOT NULL CHECK (visibility IN ('public', 'org', 'private')),
    vcs_type            text NOT NULL,
    default_branch      text NOT NULL,
    description         text,
    fork_of_repo_id     uuid REFERENCES repos (id) ON DELETE SET NULL,
    fork_of_remote      text,
    archived            boolean NOT NULL DEFAULT false,
    mirror_enabled      boolean NOT NULL DEFAULT false,
    refs_version        bigint NOT NULL DEFAULT 0,
    created_at          timestamptz NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);

-- Refs are the durable truth. Hash-partitioned by repo_id (also the future
-- shard key). Partition count is fixed at 16 for now; changing it is a
-- data move, so pick with care.
CREATE TABLE repo_refs (
    repo_id         uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    name            text NOT NULL,
    target          bytea NOT NULL,
    updated_at      timestamptz NOT NULL DEFAULT now(),
    updated_by      uuid REFERENCES identities (id) ON DELETE SET NULL,
    PRIMARY KEY (repo_id, name)
) PARTITION BY HASH (repo_id);

DO $$
BEGIN
    FOR i IN 0..15 LOOP
        EXECUTE format(
            'CREATE TABLE repo_refs_p%s PARTITION OF repo_refs FOR VALUES WITH (MODULUS 16, REMAINDER %s)',
            i, i);
    END LOOP;
END $$;

CREATE TABLE repo_segments (
    repo_id         uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    ordinal         integer NOT NULL,
    object_key      text NOT NULL,
    size            bigint NOT NULL,
    content_hash    text NOT NULL,
    kind            text NOT NULL CHECK (kind IN ('pack', 'bundle', 'snapshot')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    superseded_at   timestamptz,
    PRIMARY KEY (repo_id, ordinal)
);

CREATE TABLE repo_grants (
    repo_id         uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    grantee_kind    text NOT NULL CHECK (grantee_kind IN ('identity', 'team')),
    grantee_id      uuid NOT NULL,
    role            text NOT NULL CHECK (role IN ('admin', 'maintain', 'write', 'read')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (repo_id, grantee_kind, grantee_id)
);
CREATE INDEX repo_grants_grantee_idx ON repo_grants (grantee_kind, grantee_id);

CREATE TABLE branch_protections (
    id                          uuid PRIMARY KEY,
    repo_id                     uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    ref_pattern                 text NOT NULL,
    required_approvals          integer NOT NULL DEFAULT 0,
    dismiss_stale               boolean NOT NULL DEFAULT false,
    required_signature_level    text NOT NULL DEFAULT 'none'
                                CHECK (required_signature_level IN ('none', 'any', 'attested')),
    required_checks             text[] NOT NULL DEFAULT '{}',
    allowed_merge_kinds         text[] NOT NULL DEFAULT '{merge,squash,rebase}',
    allow_force_push            boolean NOT NULL DEFAULT false,
    allow_delete                boolean NOT NULL DEFAULT false,
    UNIQUE (repo_id, ref_pattern)
);
