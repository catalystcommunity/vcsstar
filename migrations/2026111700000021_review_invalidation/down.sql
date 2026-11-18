DROP TRIGGER pull_requests_invalidate_reviews ON pull_requests;
DROP FUNCTION invalidate_pr_reviews();
ALTER TABLE pr_reviews DROP COLUMN head_invalidated, DROP COLUMN base_invalidated;
