PRAGMA foreign_keys = OFF;

CREATE TABLE IF NOT EXISTS websites (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    key        TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS website_origins (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    website_id INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    origin     TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(website_id, origin),
    UNIQUE(origin)
);

CREATE TABLE IF NOT EXISTS website_admins (
    website_id INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (website_id, user_id)
);

CREATE TABLE IF NOT EXISTS pages (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    website_id     INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    key            TEXT NOT NULL,
    title          TEXT NOT NULL,
    url            TEXT NOT NULL DEFAULT '',
    normalized_url TEXT NOT NULL DEFAULT '',
    metadata       TEXT,
    comment_count  INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at     TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(website_id, key)
);

CREATE TABLE comments_v2 (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    website_id        INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    page_id           INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    parent_comment_id INTEGER REFERENCES comments_v2(id),
    body              TEXT NOT NULL,
    user_id           INTEGER NOT NULL REFERENCES users(id),
    reactions         TEXT NOT NULL DEFAULT '{}',
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at        TEXT
);

CREATE TABLE IF NOT EXISTS website_bans (
    website_id        INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason            TEXT,
    banned_by_user_id INTEGER REFERENCES users(id),
    banned_at         TEXT NOT NULL DEFAULT (datetime('now')),
    unbanned_at       TEXT,
    PRIMARY KEY (website_id, user_id)
);

INSERT INTO websites (id, key, name, created_at, updated_at)
SELECT
    r.id,
    lower(replace(CASE WHEN lower(r.owner) = '_global' THEN r.name ELSE r.owner || '-' || r.name END, '.', '-')),
    CASE WHEN lower(r.owner) = '_global' THEN r.name ELSE r.owner || '/' || r.name END,
    r.created_at,
    r.created_at
FROM repos r;

INSERT OR IGNORE INTO website_admins (website_id, user_id, created_at)
SELECT id, admin_user_id, created_at FROM repos WHERE admin_user_id IS NOT NULL;

INSERT OR IGNORE INTO website_admins (website_id, user_id, created_at)
SELECT id, owner_user_id, created_at FROM repos WHERE owner_user_id IS NOT NULL;

INSERT INTO pages (id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at)
SELECT
    i.id,
    i.repo_id,
    COALESCE(i.slug, 'issue-' || i.number),
    i.title,
    '',
    '',
    CASE WHEN i.body IS NULL THEN NULL ELSE json_object('legacy_body', i.body, 'legacy_number', i.number, 'legacy_state', i.state) END,
    i.comment_count,
    i.created_at,
    i.updated_at
FROM issues i;

INSERT INTO comments_v2 (id, website_id, page_id, parent_comment_id, body, user_id, reactions, created_at, updated_at, deleted_at)
SELECT
    c.id,
    c.repo_id,
    c.issue_id,
    NULL,
    c.body,
    c.user_id,
    CASE
        WHEN c.reactions IS NULL OR c.reactions = '{}' THEN '{}'
        ELSE json_object(
            'like', COALESCE(json_extract(c.reactions, '$.plus_one'), 0),
            'dislike', COALESCE(json_extract(c.reactions, '$.minus_one'), 0),
            'heart', COALESCE(json_extract(c.reactions, '$.heart'), 0),
            'laugh', COALESCE(json_extract(c.reactions, '$.laugh'), 0),
            'hooray', COALESCE(json_extract(c.reactions, '$.hooray'), 0),
            'confused', COALESCE(json_extract(c.reactions, '$.confused'), 0),
            'rocket', COALESCE(json_extract(c.reactions, '$.rocket'), 0),
            'eyes', COALESCE(json_extract(c.reactions, '$.eyes'), 0),
            'total', COALESCE(json_extract(c.reactions, '$.total'), 0)
        )
    END,
    c.created_at,
    c.updated_at,
    c.deleted_at
FROM comments c;

DROP TABLE IF EXISTS issue_labels;
DROP TABLE IF EXISTS labels;
DROP TABLE IF EXISTS comments;
DROP TABLE IF EXISTS issues;
DROP TABLE IF EXISTS repos;

ALTER TABLE comments_v2 RENAME TO comments;

CREATE TABLE IF NOT EXISTS comment_reactions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content    TEXT NOT NULL CHECK(content IN ('like','dislike','heart','laugh','hooray','confused','rocket','eyes')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(comment_id, user_id, content)
);

INSERT INTO comment_reactions (id, comment_id, user_id, content, created_at)
SELECT
    id,
    comment_id,
    user_id,
    CASE content WHEN '+1' THEN 'like' WHEN '-1' THEN 'dislike' ELSE content END,
    created_at
FROM reactions
WHERE content IN ('+1','-1','heart','laugh','hooray','confused','rocket','eyes');

DROP TABLE IF EXISTS reactions;

CREATE INDEX IF NOT EXISTS idx_website_admins_user ON website_admins(user_id);
CREATE INDEX IF NOT EXISTS idx_pages_website_key ON pages(website_id, key);
CREATE INDEX IF NOT EXISTS idx_pages_website_updated ON pages(website_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_comments_page_parent ON comments(page_id, parent_comment_id, id);
CREATE INDEX IF NOT EXISTS idx_comments_website_user ON comments(website_id, user_id);
CREATE INDEX IF NOT EXISTS idx_comment_reactions_comment ON comment_reactions(comment_id);
CREATE INDEX IF NOT EXISTS idx_website_bans_user ON website_bans(user_id, unbanned_at);

PRAGMA foreign_keys = ON;
