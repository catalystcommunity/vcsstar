ALTER TABLE pr_reviews ADD COLUMN head_invalidated boolean NOT NULL DEFAULT false;
ALTER TABLE pr_reviews ADD COLUMN base_invalidated boolean NOT NULL DEFAULT false;
CREATE FUNCTION invalidate_pr_reviews() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.head_sha IS DISTINCT FROM OLD.head_sha THEN
        UPDATE pr_reviews SET head_invalidated=true WHERE pr_id=NEW.id;
    END IF;
    IF (NEW.base_sha,NEW.base_ref) IS DISTINCT FROM (OLD.base_sha,OLD.base_ref) THEN
        UPDATE pr_reviews SET base_invalidated=true WHERE pr_id=NEW.id;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER pull_requests_invalidate_reviews AFTER UPDATE ON pull_requests
FOR EACH ROW EXECUTE FUNCTION invalidate_pr_reviews();
