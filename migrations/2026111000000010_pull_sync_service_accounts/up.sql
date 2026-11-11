-- Pull-based sync watermark: the newest GitHub `updated_at` the pull
-- request reconcile pass has processed for the repository.
ALTER TABLE mirror_state ADD COLUMN last_pull_sync_at timestamptz;

-- Service accounts: identities that never sign in through LinkKeys and
-- exist only to hold API tokens. The identity row carries the domain
-- `svc.<instance host>`; this table names the owner organization.
CREATE TABLE service_accounts (
    identity_id uuid PRIMARY KEY REFERENCES identities(id),
    org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name text NOT NULL,
    description text,
    created_by uuid REFERENCES identities(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);
