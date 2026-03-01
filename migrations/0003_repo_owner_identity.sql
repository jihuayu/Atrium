ALTER TABLE repos
ADD COLUMN owner_user_id INTEGER REFERENCES users(id);

CREATE INDEX IF NOT EXISTS idx_repos_owner_user_id ON repos(owner_user_id);

UPDATE repos
SET owner_user_id = (
    SELECT u.id
    FROM users u
    JOIN user_identities ui ON ui.user_id = u.id AND ui.provider = 'github'
    WHERE lower(u.login) = lower(repos.owner)
    LIMIT 1
)
WHERE owner_user_id IS NULL;
