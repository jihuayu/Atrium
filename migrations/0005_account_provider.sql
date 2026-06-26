PRAGMA foreign_keys = OFF;

ALTER TABLE user_identities RENAME TO user_identities_v1;

CREATE TABLE user_identities (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK(provider IN ('github','google','apple','account')),
    provider_user_id TEXT NOT NULL,
    email            TEXT NOT NULL DEFAULT '',
    avatar_url       TEXT NOT NULL DEFAULT '',
    cached_at        TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(provider, provider_user_id)
);

INSERT INTO user_identities (id, user_id, provider, provider_user_id, email, avatar_url, cached_at)
SELECT id, user_id, provider, provider_user_id, email, avatar_url, cached_at
FROM user_identities_v1;

DROP TABLE user_identities_v1;

CREATE INDEX idx_user_identities_user  ON user_identities(user_id);
CREATE INDEX idx_user_identities_email ON user_identities(email);

ALTER TABLE token_cache RENAME TO token_cache_v1;

CREATE TABLE token_cache (
    token_hash TEXT NOT NULL,
    provider   TEXT NOT NULL DEFAULT 'github'
               CHECK(provider IN ('github','google','apple','xtalk','account')),
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cached_at  TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    PRIMARY KEY (token_hash, provider)
);

INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at)
SELECT token_hash, provider, user_id, cached_at, expires_at
FROM token_cache_v1;

DROP TABLE token_cache_v1;

CREATE INDEX idx_token_cache_expires ON token_cache(expires_at);

PRAGMA foreign_keys = ON;
