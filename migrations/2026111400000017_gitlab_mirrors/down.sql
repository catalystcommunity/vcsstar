DROP INDEX credentials_target_idx;
ALTER TABLE mirror_state DROP COLUMN host, DROP COLUMN provider;
