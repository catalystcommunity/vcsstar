ALTER TABLE branch_protections ADD COLUMN require_pull_request boolean NOT NULL DEFAULT false;
ALTER TABLE branch_protections ADD COLUMN merge_authority text NOT NULL DEFAULT 'local';
ALTER TABLE branch_protections ADD COLUMN override_accounts text[] NOT NULL DEFAULT '{}';
ALTER TABLE branch_protections ADD COLUMN allow_self_approval boolean NOT NULL DEFAULT false;
ALTER TABLE branch_protections ADD COLUMN require_base_approval boolean NOT NULL DEFAULT true;
ALTER TABLE branch_protections ADD COLUMN allowed_approvers text[] NOT NULL DEFAULT '{}';
ALTER TABLE branch_protections ADD COLUMN required_check_reporters text[] NOT NULL DEFAULT '{}';
ALTER TABLE branch_protections ADD COLUMN allow_delegated_approval boolean NOT NULL DEFAULT false;
ALTER TABLE pr_reviews ADD COLUMN base_sha_at_review text;
ALTER TABLE pr_reviews ADD COLUMN base_ref_at_review text;
ALTER TABLE pr_reviews ADD COLUMN acting_via text;
ALTER TABLE pr_reviews ADD COLUMN approval_scope text;
ALTER TABLE statuses ADD COLUMN base_sha text;
CREATE FUNCTION seed_branch_protections() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO branch_protections(id,repo_id,ref_pattern,required_approvals,dismiss_stale,required_signature_level,required_checks,allowed_merge_kinds,allow_force_push,allow_delete,require_pull_request)
    SELECT gen_random_uuid(),NEW.id,pattern,1,true,'none','{}',ARRAY['merge','squash','rebase'],false,false,true
    FROM unnest(ARRAY['refs/heads/main','refs/heads/master','refs/heads/dev','refs/heads/stage','refs/heads/default','@default']) AS pattern
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END $$;
CREATE TRIGGER repos_default_protections AFTER INSERT ON repos FOR EACH ROW EXECUTE FUNCTION seed_branch_protections();
INSERT INTO branch_protections(id,repo_id,ref_pattern,required_approvals,dismiss_stale,required_signature_level,required_checks,allowed_merge_kinds,allow_force_push,allow_delete,require_pull_request)
SELECT gen_random_uuid(),r.id,pattern,1,true,'none','{}',ARRAY['merge','squash','rebase'],false,false,true
FROM repos r CROSS JOIN unnest(ARRAY['refs/heads/main','refs/heads/master','refs/heads/dev','refs/heads/stage','refs/heads/default','@default']) AS pattern
ON CONFLICT DO NOTHING;
