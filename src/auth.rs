use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    db::{self, Database, DbValue},
    error::ApiError,
    jwt,
    types::GitHubApiUser,
    types::GitHubUser,
    Result,
};

#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
pub trait HttpClient: Send + Sync {
    async fn get_github_user(&self, token: &str) -> Result<GitHubApiUser>;
    async fn get_jwks(&self, url: &str) -> Result<UpstreamResponse>;
    async fn post_utterances_token(
        &self,
        body: &[u8],
        headers: &HashMap<String, String>,
    ) -> Result<UpstreamResponse>;
}

pub struct UpstreamResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

pub fn parse_token(header: &str) -> Option<&str> {
    let header = header.trim();
    if let Some(token) = header.strip_prefix("token ") {
        return Some(token.trim());
    }
    if let Some(token) = header.strip_prefix("Bearer ") {
        return Some(token.trim());
    }
    None
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

#[derive(Debug, Deserialize)]
struct CachedUser {
    pub id: i64,
    pub login: String,
    pub email: String,
    pub avatar_url: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub site_admin: i64,
}

impl From<CachedUser> for GitHubUser {
    fn from(value: CachedUser) -> Self {
        Self {
            id: value.id,
            login: value.login,
            email: value.email,
            avatar_url: value.avatar_url,
            r#type: value.r#type,
            site_admin: value.site_admin != 0,
        }
    }
}

pub async fn resolve_github_user(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    cache_ttl_secs: i64,
) -> Result<GitHubUser> {
    let token_hash = hash_token(token);

    if let Some(cached) = db::query_opt::<CachedUser>(
        db,
        "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin \
             FROM token_cache tc \
             JOIN users u ON tc.user_id = u.id \
             WHERE tc.token_hash = ?1 AND tc.provider = 'github' AND tc.expires_at > datetime('now')",
        &[DbValue::Text(token_hash.clone())],
    )
    .await?
    {
        return Ok(cached.into());
    }

    let gh_user = http.get_github_user(token).await?;

    let user_id = resolve_or_create_provider_user(
        db,
        "github",
        &gh_user.id.to_string(),
        &gh_user.login,
        gh_user.email.as_deref().unwrap_or(""),
        &gh_user.avatar_url,
        &gh_user.r#type,
        gh_user.site_admin,
    )
    .await?;

    db.batch(vec![
        (
            "UPDATE users SET login = ?1, email = ?2, avatar_url = ?3, type = ?4, site_admin = ?5, cached_at = datetime('now') \
             WHERE id = ?6",
            vec![
                DbValue::Text(gh_user.login.clone()),
                DbValue::Text(gh_user.email.clone().unwrap_or_default()),
                DbValue::Text(gh_user.avatar_url.clone()),
                DbValue::Text(gh_user.r#type.clone()),
                DbValue::Integer(gh_user.site_admin as i64),
                DbValue::Integer(user_id),
            ],
        ),
        (
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
             ON CONFLICT(provider, provider_user_id) DO UPDATE SET \
             user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
            vec![
                DbValue::Integer(user_id),
                DbValue::Text("github".to_string()),
                DbValue::Text(gh_user.id.to_string()),
                DbValue::Text(gh_user.email.clone().unwrap_or_default()),
                DbValue::Text(gh_user.avatar_url.clone()),
            ],
        ),
        (
            "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) \
             VALUES (?1, 'github', ?2, datetime('now'), datetime('now', '+' || ?3 || ' seconds')) \
             ON CONFLICT(token_hash, provider) DO UPDATE SET \
             user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
            vec![
                DbValue::Text(token_hash),
                DbValue::Integer(user_id),
                DbValue::Integer(cache_ttl_secs),
            ],
        ),
    ])
    .await?;

    db::query_opt::<CachedUser>(
        db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .map(Into::into)
    .ok_or_else(|| ApiError::internal("failed to resolve github user"))
}

pub async fn resolve_user(
    db: &dyn Database,
    http: &dyn HttpClient,
    token: &str,
    cache_ttl_secs: i64,
) -> Result<GitHubUser> {
    resolve_github_user(db, http, token, cache_ttl_secs).await
}

pub async fn resolve_xtalk_jwt_user(
    db: &dyn Database,
    auth_header: Option<&str>,
    jwt_secret: &[u8],
) -> Result<Option<GitHubUser>> {
    let token = bearer_from_header(auth_header)?;
    let Some(token) = token else {
        return Ok(None);
    };

    let claims = jwt::verify_jwt(&token, jwt_secret)?;
    if claims.token_type == "refresh" {
        return Err(ApiError::unauthorized());
    }
    let user_id = claims
        .sub
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized())?;

    let user = db::query_opt::<CachedUser>(
        db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .map(Into::into)
    .ok_or_else(ApiError::unauthorized)?;

    Ok(Some(user))
}

async fn resolve_or_create_provider_user(
    db: &dyn Database,
    provider: &str,
    provider_user_id: &str,
    login: &str,
    email: &str,
    avatar_url: &str,
    user_type: &str,
    site_admin: bool,
) -> Result<i64> {
    #[derive(Debug, Deserialize)]
    struct IdRow {
        id: i64,
    }

    if let Some(row) = db::query_opt::<IdRow>(
        db,
        "SELECT user_id AS id FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
        &[
            DbValue::Text(provider.to_string()),
            DbValue::Text(provider_user_id.to_string()),
        ],
    )
    .await?
    {
        return Ok(row.id);
    }

    if !email.is_empty() && !email.ends_with("privaterelay.appleid.com") {
        if let Some(row) = db::query_opt::<IdRow>(
            db,
            "SELECT id FROM users WHERE email = ?1",
            &[DbValue::Text(email.to_string())],
        )
        .await?
        {
            db.execute(
                "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
                 ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
                &[
                    DbValue::Integer(row.id),
                    DbValue::Text(provider.to_string()),
                    DbValue::Text(provider_user_id.to_string()),
                    DbValue::Text(email.to_string()),
                    DbValue::Text(avatar_url.to_string()),
                ],
            )
            .await?;
            return Ok(row.id);
        }
    }

    let unique_login = unique_login(db, login).await?;
    db.execute(
        "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        &[
            DbValue::Text(unique_login.clone()),
            DbValue::Text(email.to_string()),
            DbValue::Text(avatar_url.to_string()),
            DbValue::Text(user_type.to_string()),
            DbValue::Integer(site_admin as i64),
        ],
    )
    .await?;

    let row = db::query_opt::<IdRow>(
        db,
        "SELECT id FROM users WHERE login = ?1",
        &[DbValue::Text(unique_login)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to create provider user"))?;

    db.execute(
        "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        &[
            DbValue::Integer(row.id),
            DbValue::Text(provider.to_string()),
            DbValue::Text(provider_user_id.to_string()),
            DbValue::Text(email.to_string()),
            DbValue::Text(avatar_url.to_string()),
        ],
    )
    .await?;

    Ok(row.id)
}

async fn unique_login(db: &dyn Database, preferred: &str) -> Result<String> {
    let base = if preferred.trim().is_empty() {
        "user".to_string()
    } else {
        preferred.trim().to_lowercase()
    };

    #[derive(Debug, Deserialize)]
    struct ExistsRow {
        #[serde(rename = "hit")]
        _hit: i64,
    }

    for index in 0..1000 {
        let candidate = if index == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, index)
        };
        let exists = db::query_opt::<ExistsRow>(
            db,
            "SELECT 1 AS hit FROM users WHERE login = ?1",
            &[DbValue::Text(candidate.clone())],
        )
        .await?;
        if exists.is_none() {
            return Ok(candidate);
        }
    }

    Err(ApiError::internal("unable to allocate unique login"))
}

pub fn bearer_from_header(header: Option<&str>) -> Result<Option<String>> {
    match header {
        None => Ok(None),
        Some(h) => parse_token(h)
            .map(|v| Some(v.to_string()))
            .ok_or_else(|| ApiError::unauthorized()),
    }
}

#[cfg(any(feature = "test-utils", feature = "worker"))]
pub fn try_test_bypass(
    auth_header: &str,
    bypass_secret: Option<&str>,
) -> Option<crate::types::AuthUser> {
    let bypass_secret = bypass_secret?;
    let rest = auth_header.strip_prefix("testuser ")?;
    let mut parts = rest.splitn(4, ':');
    let secret = parts.next()?;
    if secret != bypass_secret {
        return None;
    }

    let id = parts.next()?.parse::<i64>().ok()?;
    let login = parts.next()?.to_string();
    let email = parts.next().unwrap_or("").to_string();

    Some(crate::types::AuthUser {
        id,
        login,
        email,
        avatar_url: format!("https://avatars.githubusercontent.com/u/{}?v=4", id),
        r#type: "User".to_string(),
        site_admin: false,
    })
}

#[cfg(any(feature = "test-utils", feature = "worker"))]
pub async fn upsert_auth_user(db: &dyn Database, user: &crate::types::AuthUser) -> Result<()> {
    db.execute(
        "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now')) \
         ON CONFLICT(id) DO UPDATE SET \
         login = excluded.login, email = excluded.email, avatar_url = excluded.avatar_url, \
         type = excluded.type, site_admin = excluded.site_admin, cached_at = datetime('now')",
        &[
            DbValue::Integer(user.id),
            DbValue::Text(user.login.clone()),
            DbValue::Text(user.email.clone()),
            DbValue::Text(user.avatar_url.clone()),
            DbValue::Text(user.r#type.clone()),
            DbValue::Integer(user.site_admin as i64),
        ],
    )
    .await?;

    db.execute(
        "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
         VALUES (?1, 'github', ?2, ?3, ?4, datetime('now')) \
         ON CONFLICT(provider, provider_user_id) DO UPDATE SET \
         user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
        &[
            DbValue::Integer(user.id),
            DbValue::Text(user.id.to_string()),
            DbValue::Text(user.email.clone()),
            DbValue::Text(user.avatar_url.clone()),
        ],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bearer_from_header, hash_token, parse_token};

    #[cfg(feature = "server")]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "server")]
    use async_trait::async_trait;
    #[cfg(feature = "server")]
    use bytes::Bytes;
    #[cfg(feature = "server")]
    use serde::Deserialize;

    #[cfg(feature = "server")]
    use super::{resolve_github_user, resolve_xtalk_jwt_user, HttpClient, UpstreamResponse};
    #[cfg(all(feature = "server", any(feature = "test-utils", feature = "worker")))]
    use crate::types::AuthUser;
    #[cfg(feature = "server")]
    use crate::{
        db::{self, Database, DbValue},
        error::ApiError,
        jwt,
        types::{GitHubApiUser, JwtClaims},
    };

    #[cfg(feature = "server")]
    async fn make_db() -> (
        tempfile::TempPath,
        crate::platform::server::sqlite::SqliteDatabase,
    ) {
        let db_file = tempfile::NamedTempFile::new()
            .expect("temp file")
            .into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = crate::platform::server::sqlite::SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, db)
    }

    #[cfg(feature = "server")]
    struct MockHttp {
        calls: AtomicUsize,
        user: GitHubApiUser,
    }

    #[cfg(feature = "server")]
    impl MockHttp {
        fn new(user: GitHubApiUser) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                user,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[cfg(feature = "server")]
    #[async_trait]
    impl HttpClient for MockHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.user.clone())
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &std::collections::HashMap<String, String>,
        ) -> crate::Result<UpstreamResponse> {
            Ok(UpstreamResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::new(),
            })
        }
    }

    #[test]
    fn parses_token_and_bearer() {
        assert_eq!(parse_token("token abc"), Some("abc"));
        assert_eq!(parse_token("Bearer xyz"), Some("xyz"));
        assert_eq!(parse_token("basic aaa"), None);
    }

    #[test]
    fn token_hash_is_stable() {
        let a = hash_token("ghp_example");
        let b = hash_token("ghp_example");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn bearer_from_header_variants() {
        assert_eq!(bearer_from_header(None).expect("none ok"), None);
        assert_eq!(
            bearer_from_header(Some("Bearer abc"))
                .expect("bearer ok")
                .as_deref(),
            Some("abc")
        );
        assert!(bearer_from_header(Some("Basic abc")).is_err());
    }

    #[cfg(any(feature = "test-utils", feature = "worker"))]
    #[test]
    fn test_bypass_parsing_and_secret_check() {
        let parsed = super::try_test_bypass("testuser sec:7:alice:alice@test.com", Some("sec"))
            .expect("must parse bypass");
        assert_eq!(parsed.id, 7);
        assert_eq!(parsed.login, "alice");
        assert!(
            super::try_test_bypass("testuser wrong:7:alice:alice@test.com", Some("sec")).is_none()
        );
    }

    #[cfg(any(feature = "test-utils", feature = "worker"))]
    #[cfg(feature = "server")]
    #[tokio::test]
    async fn upsert_auth_user_inserts_and_updates() {
        #[derive(Debug, Deserialize)]
        struct UserRow {
            login: String,
            email: String,
        }

        let (_db_file, db) = make_db().await;
        let user = AuthUser {
            id: 100,
            login: "alice".to_string(),
            email: "alice@a.com".to_string(),
            avatar_url: "https://x/a.png".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        super::upsert_auth_user(&db, &user)
            .await
            .expect("first upsert");

        let mut updated = user.clone();
        updated.login = "alice2".to_string();
        updated.email = "alice2@a.com".to_string();
        super::upsert_auth_user(&db, &updated)
            .await
            .expect("second upsert");

        let row = db::query_opt::<UserRow>(
            &db,
            "SELECT login, email FROM users WHERE id = ?1",
            &[DbValue::Integer(100)],
        )
        .await
        .expect("query user")
        .expect("row exists");
        assert_eq!(row.login, "alice2");
        assert_eq!(row.email, "alice2@a.com");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn resolve_github_user_creates_and_hits_cache() {
        #[derive(Debug, Deserialize)]
        struct TokenRow {
            provider: String,
        }

        let (_db_file, db) = make_db().await;
        let http = MockHttp::new(GitHubApiUser {
            id: 42,
            login: "Alice".to_string(),
            email: Some("alice@example.com".to_string()),
            avatar_url: "https://avatars/a".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        });

        let first = resolve_github_user(&db, &http, "gh-token", 3600)
            .await
            .expect("first resolve");
        assert_eq!(first.login, "Alice");
        assert_eq!(http.calls(), 1);

        let second = resolve_github_user(&db, &http, "gh-token", 3600)
            .await
            .expect("cached resolve");
        assert_eq!(second.id, first.id);
        assert_eq!(http.calls(), 1);

        let row = db::query_opt::<TokenRow>(
            &db,
            "SELECT provider FROM token_cache WHERE token_hash = ?1",
            &[DbValue::Text(hash_token("gh-token"))],
        )
        .await
        .expect("query cache")
        .expect("cache row");
        assert_eq!(row.provider, "github");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn resolve_xtalk_jwt_user_requires_access_token() {
        let (_db_file, db) = make_db().await;
        db.execute(
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            &[
                DbValue::Integer(7),
                DbValue::Text("alice".to_string()),
                DbValue::Text("alice@test.com".to_string()),
                DbValue::Text("https://avatars/a".to_string()),
                DbValue::Text("User".to_string()),
                DbValue::Integer(0),
            ],
        )
        .await
        .expect("insert user");

        let now = chrono::Utc::now().timestamp();
        let refresh = jwt::sign_jwt(
            &JwtClaims {
                sub: "7".to_string(),
                login: "alice".to_string(),
                iss: "xtalk".to_string(),
                iat: now,
                exp: now + 3600,
                jti: "j1".to_string(),
                token_type: "refresh".to_string(),
            },
            b"test-jwt-secret-at-least-32-bytes!!",
        )
        .expect("sign refresh");
        let access = jwt::sign_jwt(
            &JwtClaims {
                sub: "7".to_string(),
                login: "alice".to_string(),
                iss: "xtalk".to_string(),
                iat: now,
                exp: now + 3600,
                jti: "j2".to_string(),
                token_type: "access".to_string(),
            },
            b"test-jwt-secret-at-least-32-bytes!!",
        )
        .expect("sign access");

        let err = resolve_xtalk_jwt_user(
            &db,
            Some(&format!("Bearer {}", refresh)),
            b"test-jwt-secret-at-least-32-bytes!!",
        )
        .await
        .err()
        .expect("refresh must fail");
        assert_eq!(err.status, 401);

        let user = resolve_xtalk_jwt_user(
            &db,
            Some(&format!("Bearer {}", access)),
            b"test-jwt-secret-at-least-32-bytes!!",
        )
        .await
        .expect("access must pass")
        .expect("user exists");
        assert_eq!(user.id, 7);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn resolve_user_wrapper_and_provider_user_paths() {
        #[derive(Debug, Deserialize)]
        struct IdentityRow {
            user_id: i64,
        }

        let (_db_file, db) = make_db().await;
        db.execute(
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES \
             (11, 'existing', 'e@test.com', 'https://avatars/e', 'User', 0, datetime('now'))",
            &[],
        )
        .await
        .expect("insert existing user");
        db.execute(
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) VALUES \
             (11, 'google', 'g-11', 'e@test.com', 'https://avatars/e', datetime('now'))",
            &[],
        )
        .await
        .expect("insert existing identity");

        let http = MockHttp::new(GitHubApiUser {
            id: 42,
            login: "Alice".to_string(),
            email: Some("alice@example.com".to_string()),
            avatar_url: "https://avatars/a".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        });
        let resolved = super::resolve_user(&db, &http, "gh-token-wrapper", 3600)
            .await
            .expect("resolve via wrapper");
        assert_eq!(resolved.login, "Alice");

        let by_identity = super::resolve_or_create_provider_user(
            &db,
            "google",
            "g-11",
            "new-login",
            "x@test.com",
            "https://avatars/x",
            "User",
            false,
        )
        .await
        .expect("identity hit");
        assert_eq!(by_identity, 11);

        let by_email = super::resolve_or_create_provider_user(
            &db,
            "apple",
            "apple-11",
            "new-login",
            "e@test.com",
            "https://avatars/e2",
            "User",
            false,
        )
        .await
        .expect("email match");
        assert_eq!(by_email, 11);

        let identity = db::query_opt::<IdentityRow>(
            &db,
            "SELECT user_id FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
            &[
                DbValue::Text("apple".to_string()),
                DbValue::Text("apple-11".to_string()),
            ],
        )
        .await
        .expect("query identity")
        .expect("identity exists");
        assert_eq!(identity.user_id, 11);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn unique_login_handles_empty_preferred_and_exhaustion() {
        let (_db_file, db) = make_db().await;

        db.execute(
            "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, '', '', 'User', 0, datetime('now'))",
            &[DbValue::Text("user".to_string())],
        )
        .await
        .expect("insert user");
        let candidate = super::unique_login(&db, "   ")
            .await
            .expect("allocate user-1");
        assert_eq!(candidate, "user-1");

        for index in 1..1000 {
            let login = format!("user-{}", index);
            db.execute(
                "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, '', '', 'User', 0, datetime('now'))",
                &[DbValue::Text(login)],
            )
            .await
            .expect("insert login");
        }

        let err = super::unique_login(&db, "user")
            .await
            .err()
            .expect("must exhaust");
        assert_eq!(err.status, 500);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn mock_http_aux_methods_are_exercised() {
        let http = MockHttp::new(GitHubApiUser {
            id: 1,
            login: "u".to_string(),
            email: None,
            avatar_url: String::new(),
            r#type: "User".to_string(),
            site_admin: false,
        });
        let jwks_err = http
            .get_jwks("https://example.com/jwks")
            .await
            .err()
            .expect("not used");
        assert_eq!(jwks_err.status, 500);
        let up = http
            .post_utterances_token(&[], &std::collections::HashMap::new())
            .await
            .expect("ok");
        assert_eq!(up.status, 200);
    }
}
