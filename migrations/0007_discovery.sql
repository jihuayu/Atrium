CREATE TABLE IF NOT EXISTS website_pending_admins (
    website_id      INTEGER NOT NULL REFERENCES websites(id) ON DELETE CASCADE,
    email           TEXT NOT NULL,
    source          TEXT NOT NULL DEFAULT 'discovery',
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    claimed_at      TEXT,
    claimed_user_id INTEGER REFERENCES users(id),
    PRIMARY KEY (website_id, email)
);

CREATE TABLE IF NOT EXISTS website_discovery_cache (
    origin      TEXT PRIMARY KEY,
    status      TEXT NOT NULL CHECK(status IN ('not_found','invalid','error','conflict','discovered')),
    website_id  INTEGER REFERENCES websites(id) ON DELETE SET NULL,
    error       TEXT,
    source      TEXT,
    checked_at  TEXT NOT NULL DEFAULT (datetime('now')),
    retry_after TEXT
);

CREATE INDEX IF NOT EXISTS idx_website_pending_admins_email ON website_pending_admins(email, claimed_at);
CREATE INDEX IF NOT EXISTS idx_website_discovery_cache_retry ON website_discovery_cache(retry_after);
