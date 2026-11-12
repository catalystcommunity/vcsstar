-- History import of GitHub pull requests. One cursor per mirrored
-- repository: the page and the position in it of GitHub's pull request
-- list in creation order (`sort=created&direction=asc`, which only grows
-- at the end). `backfill_done_at` NULL means the import is not complete.
-- `backfill_pass_at` is the claim and the schedule (oldest first).
ALTER TABLE mirror_state
    ADD COLUMN backfill_page integer NOT NULL DEFAULT 1,
    ADD COLUMN backfill_index integer NOT NULL DEFAULT 0,
    ADD COLUMN backfill_pulls integer NOT NULL DEFAULT 0,
    ADD COLUMN backfill_heads_at timestamptz,
    ADD COLUMN backfill_pass_at timestamptz,
    ADD COLUMN backfill_started_at timestamptz,
    ADD COLUMN backfill_done_at timestamptz;

-- The instance can have several GitHub Apps. A link remembers the App
-- whose client issued its token, because a refresh must use that client.
-- NULL: the first App (links made before this column).
ALTER TABLE github_links ADD COLUMN app_id text;
