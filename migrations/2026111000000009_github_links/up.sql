-- A vcsstar identity linked to a GitHub account through the instance's
-- GitHub App (user authorization) or a personal access token the user
-- pasted. The tokens are sealed with the instance credential key. With a
-- link, vcsstar acts on GitHub as that user and attributes what the user
-- does on GitHub to the vcsstar identity.
CREATE TABLE github_links (
    identity_id         uuid PRIMARY KEY REFERENCES identities (id) ON DELETE CASCADE,
    github_id           bigint NOT NULL UNIQUE,
    login               text NOT NULL,
    -- 'oauth' (GitHub App user token, refreshable) or 'pat'
    kind                text NOT NULL,
    access_ciphertext   bytea NOT NULL,
    access_expires_at   timestamptz,
    refresh_ciphertext  bytea,
    refresh_expires_at  timestamptz,
    -- Set when GitHub refused the token and a refresh did not help; the
    -- user has to connect again.
    broken_at           timestamptz,
    broken_reason       text,
    linked_at           timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now(),
    last_used_at        timestamptz
);

-- One browser round trip to GitHub's authorize page. The state value is
-- single use and short lived.
CREATE TABLE github_link_attempts (
    state_hash      bytea PRIMARY KEY,
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    return_to       text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL
);

-- Where an event came from. 'github' marks every event written while a
-- GitHub delivery was applied; the dispatcher never mirrors those back,
-- whoever the actor is.
ALTER TABLE events ADD COLUMN origin text;
