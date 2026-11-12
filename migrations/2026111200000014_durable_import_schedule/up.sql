-- Imports run in the mirror worker from a corndogs task (queue
-- `import:github`), not in the request. The row is made paused and not
-- enabled; the worker fetches the refs and enables the mirror. A pod
-- restart only delays the task. `import_claimed_at` keeps two workers
-- from importing one repository at once.
ALTER TABLE mirror_state
    ADD COLUMN import_requested_by uuid REFERENCES identities (id),
    ADD COLUMN import_requested_at timestamptz,
    ADD COLUMN import_claimed_at timestamptz,
    ADD COLUMN import_attempts integer NOT NULL DEFAULT 0,
    ADD COLUMN import_done_at timestamptz;

UPDATE mirror_state m SET import_done_at = m.created_at
    FROM repos r WHERE r.id = m.repo_id AND r.mirror_enabled;

-- Per-repository cadence of the pull-based passes, in seconds.
-- NULL: the instance default (15 minutes each).
ALTER TABLE mirror_state
    ADD COLUMN pull_sync_interval_s integer
        CHECK (pull_sync_interval_s BETWEEN 60 AND 604800),
    ADD COLUMN ref_reconcile_interval_s integer
        CHECK (ref_reconcile_interval_s BETWEEN 60 AND 604800);
