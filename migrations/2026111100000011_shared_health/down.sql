DROP INDEX hook_deliveries_event_uidx;
ALTER TABLE pull_requests DROP COLUMN github_updated_at;
ALTER TABLE pr_comments DROP COLUMN github_updated_at;
ALTER TABLE mirror_state DROP COLUMN last_pull_pass_at;
DROP TABLE github_rate_limits;
DROP TABLE github_lanes;
