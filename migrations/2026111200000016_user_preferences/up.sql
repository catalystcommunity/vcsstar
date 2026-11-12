-- Display preferences per identity (Auth/get-preferences,
-- Auth/set-preferences). A missing row means the defaults.
CREATE TABLE user_preferences (
    identity_id uuid PRIMARY KEY REFERENCES identities (id) ON DELETE CASCADE,
    diff_layout text NOT NULL DEFAULT 'split' CHECK (diff_layout IN ('split', 'unified')),
    updated_at timestamptz NOT NULL DEFAULT now()
);
