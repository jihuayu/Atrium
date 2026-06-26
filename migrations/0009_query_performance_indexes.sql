CREATE INDEX IF NOT EXISTS idx_comments_website_page_parent_id ON comments(website_id, page_id, parent_comment_id, id);
CREATE INDEX IF NOT EXISTS idx_comments_website_id ON comments(website_id, id);
