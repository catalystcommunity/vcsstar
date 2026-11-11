ALTER TABLE repo_segments DROP COLUMN has_bitmap;
ALTER TABLE repos DROP COLUMN repacked_at;
-- The partitioned shape is not restored; the plain table stays.
