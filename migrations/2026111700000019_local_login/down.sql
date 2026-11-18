DROP TRIGGER identities_linkkeys_binding ON identities;
DROP FUNCTION keep_linkkeys_binding();
DROP TABLE local_link_attempts;
DROP TABLE local_accounts;
ALTER TABLE identities DROP COLUMN linkkeys_bound;
