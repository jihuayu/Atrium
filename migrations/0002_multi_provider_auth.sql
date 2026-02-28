ALTER TABLE users RENAME TO users_v1;

CREATE TABLE users (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    login      TEXT NOT NULL UNIQUE,
    email      TEXT NOT NULL DEFAULT '',
    avatar_url TEXT NOT NULL DEFAULT '',
    type       TEXT NOT NULL DEFAULT 'User',
    site_admin INTEGER NOT NULL DEFAULT 0,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO users SELECT id, login, email, avatar_url, type, site_admin, cached_at FROM users_v1;

CREATE TABLE user_identities (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK(provider IN ('github','google','apple')),
    provider_user_id TEXT NOT NULL,
    email            TEXT NOT NULL DEFAULT '',
    avatar_url       TEXT NOT NULL DEFAULT '',
    cached_at        TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX idx_user_identities_user  ON user_identities(user_id);
CREATE INDEX idx_user_identities_email ON user_identities(email);

INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at)
SELECT id, 'github', CAST(id AS TEXT), email, avatar_url, cached_at FROM users_v1;

ALTER TABLE token_cache RENAME TO token_cache_v1;

CREATE TABLE token_cache (
    token_hash TEXT NOT NULL,
    provider   TEXT NOT NULL DEFAULT 'github'
               CHECK(provider IN ('github','google','apple','xtalk')),
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    PRIMARY KEY (token_hash, provider)
);

CREATE INDEX idx_token_cache_expires ON token_cache(expires_at);

INSERT INTO token_cache
SELECT token_hash, 'github', user_id, cached_at, expires_at FROM token_cache_v1;

CREATE TABLE sessions (
    refresh_token_hash TEXT PRIMARY KEY,
    user_id            INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at         TEXT NOT NULL,
    revoked_at         TEXT
);

CREATE INDEX idx_sessions_user    ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

CREATE TABLE jwks_cache (
    provider   TEXT PRIMARY KEY,
    jwks_json  TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

DROP TABLE users_v1;
DROP TABLE token_cache_v1;