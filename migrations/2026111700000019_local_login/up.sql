-- A LinkKeys binding is permanent for the lifetime of an identity.
ALTER TABLE identities ADD COLUMN linkkeys_bound boolean NOT NULL DEFAULT true;
CREATE TABLE local_accounts (
    identity_id uuid PRIMARY KEY REFERENCES identities(id) ON DELETE CASCADE,
    username text NOT NULL UNIQUE CHECK (username = lower(username)),
    password_hash text,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE local_link_attempts (
    attempt_hash bytea PRIMARY KEY,
    identity_id uuid NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE FUNCTION keep_linkkeys_binding() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.linkkeys_bound AND NOT NEW.linkkeys_bound THEN
        RAISE EXCEPTION 'a LinkKeys identity cannot enable local login';
    END IF;
    IF (NEW.lk_user_id, NEW.lk_domain) IS DISTINCT FROM (OLD.lk_user_id, OLD.lk_domain) THEN
        NEW.linkkeys_bound := true;
    END IF;
    IF NEW.linkkeys_bound AND NOT OLD.linkkeys_bound THEN
        UPDATE local_accounts SET password_hash = NULL, updated_at = now() WHERE identity_id = NEW.id;
        DELETE FROM sessions WHERE identity_id = NEW.id;
        UPDATE api_tokens SET revoked_at = now() WHERE identity_id = NEW.id AND revoked_at IS NULL;
        UPDATE ssh_keys SET revoked_at = now() WHERE identity_id = NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER identities_linkkeys_binding BEFORE UPDATE ON identities
FOR EACH ROW EXECUTE FUNCTION keep_linkkeys_binding();
