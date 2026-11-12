ALTER TABLE github_links DROP COLUMN app_id;
ALTER TABLE mirror_state
    DROP COLUMN backfill_page,
    DROP COLUMN backfill_index,
    DROP COLUMN backfill_pulls,
    DROP COLUMN backfill_heads_at,
    DROP COLUMN backfill_pass_at,
    DROP COLUMN backfill_started_at,
    DROP COLUMN backfill_done_at;
