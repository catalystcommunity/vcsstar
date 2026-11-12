-- Roles on the whole instance, beside `INSTANCE_ADMINS` (configuration).
-- "admin": manage everything. "viewer": read every repository.
-- `source`: "manual" (set by an admin through Instance/set-role) or
-- "claims" (from a LinkKeys claim policy; reserved for later).
CREATE TABLE instance_roles (
    identity_id uuid PRIMARY KEY REFERENCES identities (id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('admin', 'viewer')),
    source text NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'claims')),
    granted_by uuid REFERENCES identities (id),
    granted_at timestamptz NOT NULL DEFAULT now()
);
