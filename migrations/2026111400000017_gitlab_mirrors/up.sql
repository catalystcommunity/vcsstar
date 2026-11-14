ALTER TABLE mirror_state
    ADD COLUMN provider text NOT NULL DEFAULT 'github'
        CHECK (provider IN ('github', 'gitlab')),
    ADD COLUMN host text NOT NULL DEFAULT 'github.com';

CREATE INDEX credentials_target_idx ON credentials (org_id, provider, host, purpose);
