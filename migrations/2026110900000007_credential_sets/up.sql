-- Credentials form a set per scope and purpose: several rows may share
-- (org_id, repo_id, provider, purpose) and an operation picks one at
-- random. The unique constraint from the mirror migration goes away.
ALTER TABLE credentials DROP CONSTRAINT credentials_org_id_repo_id_provider_purpose_key;
CREATE INDEX credentials_scope_idx ON credentials (org_id, repo_id, purpose) WHERE enabled;
