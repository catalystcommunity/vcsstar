ALTER TABLE mirror_state
    DROP COLUMN import_requested_by,
    DROP COLUMN import_requested_at,
    DROP COLUMN import_claimed_at,
    DROP COLUMN import_attempts,
    DROP COLUMN import_done_at,
    DROP COLUMN pull_sync_interval_s,
    DROP COLUMN ref_reconcile_interval_s;
