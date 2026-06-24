ALTER TABLE issues ADD COLUMN slug TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_repo_slug
    ON issues(repo_id, slug)
    WHERE slug IS NOT NULL AND deleted_at IS NULL;
