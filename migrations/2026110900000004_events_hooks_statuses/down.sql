DROP TABLE statuses;
DROP TABLE hook_deliveries;
DROP TABLE hook_endpoints;
DROP INDEX events_undispatched_idx;
ALTER TABLE events DROP COLUMN dispatched_at;
