-- Bidirectional GitHub sync for pull requests, reviews, comments, and
-- statuses. vcsstar rows remember the GitHub object they shadow (or that
-- shadows them), so a webhook about an object we created is recognised and
-- an object we imported is never sent back.

ALTER TABLE pull_requests ADD COLUMN github_number integer;
CREATE UNIQUE INDEX pull_requests_github_idx
    ON pull_requests (repo_id, github_number) WHERE github_number IS NOT NULL;

ALTER TABLE pr_reviews ADD COLUMN github_id bigint;
CREATE UNIQUE INDEX pr_reviews_github_idx
    ON pr_reviews (pr_id, github_id) WHERE github_id IS NOT NULL;

ALTER TABLE pr_comments ADD COLUMN github_id bigint;
-- 'issue' (conversation) or 'review' (inline on a diff line) on GitHub.
ALTER TABLE pr_comments ADD COLUMN github_kind text;
CREATE UNIQUE INDEX pr_comments_github_idx
    ON pr_comments (pr_id, github_id) WHERE github_id IS NOT NULL;

-- Shadow identities for GitHub accounts that act on a mirrored repository.
-- They never log in; events they cause are not sent back to GitHub.
CREATE TABLE github_identities (
    identity_id     uuid PRIMARY KEY REFERENCES identities (id) ON DELETE CASCADE,
    github_id       bigint NOT NULL UNIQUE,
    login           text NOT NULL,
    updated_at      timestamptz NOT NULL DEFAULT now()
);
