-- Combined native schema for Worker tests.
-- Used by scripts/test-worker.ts for fresh D1 test databases.

CREATE TABLE IF NOT EXISTS users (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    login      TEXT NOT NULL UNIQUE,
    email      TEXT NOT NULL DEFAULT '',
    avatar_url TEXT NOT NULL DEFAULT '',
    type       TEXT NOT NULL DEFAULT 'User',
    site_admin INTEGER NOT NULL DEFAULT 0,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS user_identities (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK(provider IN ('github','google','apple','account')),
    provider_user_id TEXT NOT NULL,
    email            TEXT NOT NULL DEFAULT '',
    avatar_url       TEXT NOT NULL DEFAULT '',
    cached_at        TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX IF NOT EXISTS idx_user_identities_user  ON user_identities(user_id);
CREATE INDEX IF NOT EXISTS idx_user_identities_email ON user_identities(email);

CREATE TABLE IF NOT EXISTS token_cache (
    token_hash TEXT NOT NULL,
    provider   TEXT NOT NULL DEFAULT 'account'
               CHECK(provider IN ('github','google','apple','xtalk','account')),
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    PRIMARY KEY (token_hash, provider)
);

CREATE INDEX IF NOT EXISTS idx_token_cache_expires ON token_cache(expires_at);

CREATE TABLE IF NOT EXISTS sessions (
    refresh_token_hash TEXT PRIMARY KEY,
    user_id            INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at         TEXT NOT NULL,
    revoked_at         TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_user    ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);

CREATE TABLE IF NOT EXISTS jwks_cache (
    provider   TEXT PRIMARY KEY,
    jwks_json  TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

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

CREATE TABLE IF NOT EXISTS comments (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    website_id        INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    page_id           INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    parent_comment_id INTEGER REFERENCES comments(id),
    body              TEXT NOT NULL,
    user_id           INTEGER NOT NULL REFERENCES users(id),
    reactions         TEXT NOT NULL DEFAULT '{}',
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at        TEXT
);

CREATE TABLE IF NOT EXISTS comment_reactions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content    TEXT NOT NULL CHECK(content IN ('like','dislike','heart','laugh','hooray','confused','rocket','eyes')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(comment_id, user_id, content)
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

CREATE INDEX IF NOT EXISTS idx_website_admins_user ON website_admins(user_id);
CREATE INDEX IF NOT EXISTS idx_pages_website_key ON pages(website_id, key);
CREATE INDEX IF NOT EXISTS idx_pages_website_updated ON pages(website_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_comments_page_parent ON comments(page_id, parent_comment_id, id);
CREATE INDEX IF NOT EXISTS idx_comments_website_user ON comments(website_id, user_id);
CREATE INDEX IF NOT EXISTS idx_comment_reactions_comment ON comment_reactions(comment_id);
CREATE INDEX IF NOT EXISTS idx_website_bans_user ON website_bans(user_id, unbanned_at);
