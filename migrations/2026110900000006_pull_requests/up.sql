-- Phase 4: pull requests, reviews, comments, and the instance's own
-- LinkKeys application keys (sealed with the credential key).

CREATE TABLE pull_requests (
    id                  uuid PRIMARY KEY,
    repo_id             uuid NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    number              integer NOT NULL,
    title               text NOT NULL,
    body                text,
    author_identity_id  uuid NOT NULL REFERENCES identities (id),
    head_repo_id        uuid REFERENCES repos (id) ON DELETE SET NULL,
    head_remote         text,
    head_ref            text NOT NULL,
    base_ref            text NOT NULL,
    head_sha            text NOT NULL,
    base_sha            text NOT NULL,
    merge_base_sha      text,
    mergeable           text NOT NULL DEFAULT 'unknown' CHECK (mergeable IN ('clean', 'conflicts', 'unknown')),
    state               text NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'merged', 'closed')),
    merge_kind          text,
    merged_commit_sha   text,
    merged_by           uuid REFERENCES identities (id),
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now(),
    closed_at           timestamptz,
    UNIQUE (repo_id, number)
);
CREATE INDEX pull_requests_repo_state_idx ON pull_requests (repo_id, state, number DESC);
CREATE INDEX pull_requests_head_idx ON pull_requests (head_repo_id, head_ref) WHERE state = 'open';

-- Per-repo PR numbering.
CREATE TABLE pr_counters (
    repo_id     uuid PRIMARY KEY REFERENCES repos (id) ON DELETE CASCADE,
    next_number integer NOT NULL DEFAULT 1
);

CREATE TABLE pr_reviews (
    id                  uuid PRIMARY KEY,
    pr_id               uuid NOT NULL REFERENCES pull_requests (id) ON DELETE CASCADE,
    reviewer            uuid NOT NULL REFERENCES identities (id),
    state               text NOT NULL CHECK (state IN ('approved', 'changes_requested', 'comment')),
    head_sha_at_review  text NOT NULL,
    body                text,
    created_at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pr_reviews_pr_idx ON pr_reviews (pr_id, created_at);

CREATE TABLE pr_comments (
    id          uuid PRIMARY KEY,
    pr_id       uuid NOT NULL REFERENCES pull_requests (id) ON DELETE CASCADE,
    author      uuid NOT NULL REFERENCES identities (id),
    body        text NOT NULL,
    path        text,
    line        integer,
    commit_sha  text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    edited_at   timestamptz
);
CREATE INDEX pr_comments_pr_idx ON pr_comments (pr_id, created_at);

-- The instance's application keys (application `vcsstar`), enrolled at the
-- instance account's home domain. Private seeds are sealed with the
-- credential key. Three signing keys so any one can be revoked by two.
CREATE TABLE instance_keys (
    key_id                  text PRIMARY KEY,
    instance_id             text NOT NULL,
    role                    text NOT NULL,
    public_key              bytea NOT NULL,
    private_sealed          bytea NOT NULL,
    attested_at             timestamptz,
    attestation_expires_at  timestamptz,
    created_at              timestamptz NOT NULL DEFAULT now(),
    revoked_at              timestamptz
);
