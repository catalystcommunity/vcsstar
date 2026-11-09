-- Identity, sessions, tokens, keys. See plan/02-data-model.md.
-- Every id is a UUID v7 made by the application.

CREATE TABLE identities (
    id              uuid PRIMARY KEY,
    lk_user_id      uuid NOT NULL,
    lk_domain       text NOT NULL,
    handle          text NOT NULL,
    display_name    text,
    email           text,
    email_verified  boolean NOT NULL DEFAULT false,
    first_seen      timestamptz NOT NULL DEFAULT now(),
    last_login      timestamptz,
    disabled_at     timestamptz,
    UNIQUE (lk_user_id, lk_domain)
);
-- Handles are not unique. This index serves display lookups only.
CREATE INDEX identities_handle_domain_idx ON identities (lk_domain, handle);

CREATE TABLE identity_claims (
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    claim_type      text NOT NULL,
    claim_cbor      bytea NOT NULL,
    attested_at     timestamptz,
    expires_at      timestamptz,
    verified_at     timestamptz,
    PRIMARY KEY (identity_id, claim_type)
);

CREATE TABLE ssh_keys (
    id              uuid PRIMARY KEY,
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    usage           text NOT NULL CHECK (usage IN ('auth', 'signing')),
    algo            text NOT NULL,
    public_key      bytea NOT NULL,
    fingerprint     text NOT NULL UNIQUE,
    attested        boolean NOT NULL DEFAULT false,
    application_id  text,
    instance_id     text,
    key_id          text,
    attested_until  timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    revoked_at      timestamptz
);
CREATE INDEX ssh_keys_identity_idx ON ssh_keys (identity_id);

CREATE TABLE app_key_cache (
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    application_id  text NOT NULL,
    instance_id     text NOT NULL,
    raw_cbor        bytea NOT NULL,
    fetched_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (identity_id, application_id, instance_id)
);

CREATE TABLE act_as_grants (
    grant_id                    text PRIMARY KEY,
    identity_id                 uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    grantee                     text NOT NULL,
    scopes                      text[] NOT NULL,
    expires_at                  timestamptz NOT NULL,
    revoked_at                  timestamptz,
    last_presentation_nonce_at  timestamptz
);

CREATE TABLE act_as_nonces (
    nonce       text PRIMARY KEY,
    seen_at     timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX act_as_nonces_seen_idx ON act_as_nonces (seen_at);

CREATE TABLE sessions (
    id              uuid PRIMARY KEY,
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    token_hash      bytea NOT NULL UNIQUE,
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL,
    last_used_at    timestamptz,
    user_agent      text
);
CREATE INDEX sessions_identity_idx ON sessions (identity_id);

CREATE TABLE api_tokens (
    id              uuid PRIMARY KEY,
    identity_id     uuid NOT NULL REFERENCES identities (id) ON DELETE CASCADE,
    token_hash      bytea NOT NULL UNIQUE,
    name            text NOT NULL,
    scopes          text[] NOT NULL,
    client_label    text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz,
    revoked_at      timestamptz
);
CREATE INDEX api_tokens_identity_idx ON api_tokens (identity_id);

-- Browser-side pending logins (bound to the browser session by hash).
CREATE TABLE pending_logins (
    id                  uuid PRIMARY KEY,
    attempt_hash        bytea NOT NULL UNIQUE,
    nonce               text NOT NULL,
    identity_input      text NOT NULL,
    idp_domain          text NOT NULL,
    return_to           text,
    device_login_id     uuid,
    created_at          timestamptz NOT NULL DEFAULT now(),
    expires_at          timestamptz NOT NULL,
    claimed_at          timestamptz
);

CREATE TABLE device_logins (
    id              uuid PRIMARY KEY,
    code_hash       bytea NOT NULL UNIQUE,
    user_code_hash  bytea NOT NULL UNIQUE,
    client_label    text NOT NULL,
    scopes          text[] NOT NULL,
    state           text NOT NULL CHECK (state IN ('pending', 'approved', 'denied', 'expired', 'consumed')),
    identity_id     uuid REFERENCES identities (id) ON DELETE SET NULL,
    api_token_id    uuid REFERENCES api_tokens (id) ON DELETE SET NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL
);
CREATE INDEX device_logins_expires_idx ON device_logins (expires_at);
