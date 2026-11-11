DROP TABLE github_identities;
DROP INDEX pr_comments_github_idx;
ALTER TABLE pr_comments DROP COLUMN github_kind;
ALTER TABLE pr_comments DROP COLUMN github_id;
DROP INDEX pr_reviews_github_idx;
ALTER TABLE pr_reviews DROP COLUMN github_id;
DROP INDEX pull_requests_github_idx;
ALTER TABLE pull_requests DROP COLUMN github_number;
