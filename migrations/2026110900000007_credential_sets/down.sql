-- Fails when a scope holds more than one credential; remove the extras first.
DROP INDEX IF EXISTS credentials_scope_idx;
ALTER TABLE credentials ADD CONSTRAINT credentials_org_id_repo_id_provider_purpose_key
    UNIQUE (org_id, repo_id, provider, purpose);
