use serde::Deserialize;

use crate::{
    auth::hash_token,
    db::{self, DbValue},
    error::ApiError,
    AppContext, Result,
};

#[derive(Debug, Deserialize)]
struct SessionRow {
    user_id: i64,
    expires_at: String,
    revoked_at: Option<String>,
}

pub async fn create_session(
    ctx: &AppContext<'_>,
    refresh_token: &str,
    user_id: i64,
    ttl_secs: i64,
) -> Result<()> {
    let token_hash = hash_token(refresh_token);
    ctx.db
        .execute(
            "INSERT INTO sessions (refresh_token_hash, user_id, created_at, expires_at, revoked_at) \
             VALUES (?1, ?2, datetime('now'), datetime('now', '+' || ?3 || ' seconds'), NULL) \
             ON CONFLICT(refresh_token_hash) DO UPDATE SET \
             user_id = excluded.user_id, created_at = datetime('now'), expires_at = excluded.expires_at, revoked_at = NULL",
            &[
                DbValue::Text(token_hash),
                DbValue::Integer(user_id),
                DbValue::Integer(ttl_secs),
            ],
        )
        .await?;
    Ok(())
}

pub async fn validate_session(
    ctx: &AppContext<'_>,
    refresh_token: &str,
    user_id: i64,
) -> Result<()> {
    let token_hash = hash_token(refresh_token);
    let row = db::query_opt::<SessionRow>(
        ctx.db,
        "SELECT user_id, expires_at, revoked_at FROM sessions WHERE refresh_token_hash = ?1",
        &[DbValue::Text(token_hash)],
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;

    if row.user_id != user_id || row.revoked_at.is_some() {
        return Err(ApiError::unauthorized());
    }
    if !row.expires_at.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let expires = parse_sqlite_datetime(&row.expires_at).unwrap_or(0);
        if expires <= now {
            return Err(ApiError::unauthorized());
        }
    }

    Ok(())
}

pub async fn revoke_session(ctx: &AppContext<'_>, refresh_token: &str) -> Result<()> {
    let token_hash = hash_token(refresh_token);
    ctx.db
        .execute(
            "UPDATE sessions SET revoked_at = datetime('now') WHERE refresh_token_hash = ?1",
            &[DbValue::Text(token_hash)],
        )
        .await?;
    Ok(())
}

pub async fn revoke_user_sessions(ctx: &AppContext<'_>, user_id: i64) -> Result<()> {
    ctx.db
        .execute(
            "UPDATE sessions SET revoked_at = datetime('now') WHERE user_id = ?1 AND revoked_at IS NULL",
            &[DbValue::Integer(user_id)],
        )
        .await?;
    Ok(())
}

fn parse_sqlite_datetime(value: &str) -> Option<i64> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(parsed.timestamp());
    }
    let fixed = if value.contains('T') {
        value.to_string()
    } else {
        value.replace(' ', "T") + "Z"
    };
    chrono::DateTime::parse_from_rfc3339(&fixed)
        .ok()
        .map(|v| v.timestamp())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "server")]
    use std::collections::HashMap;

    #[cfg(feature = "server")]
    use async_trait::async_trait;
    #[cfg(feature = "server")]
    use bytes::Bytes;

    use super::parse_sqlite_datetime;

    #[cfg(feature = "server")]
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::{Database, DbValue},
        types::GitHubApiUser,
        AppContext,
    };

    #[test]
    fn parse_rfc3339_datetime() {
        let ts = parse_sqlite_datetime("2025-01-15T08:00:00Z").expect("must parse");
        assert!(ts > 0);
    }

    #[test]
    fn parse_sqlite_plain_datetime() {
        let ts = parse_sqlite_datetime("2025-01-15 08:00:00").expect("must parse");
        assert!(ts > 0);
    }

    #[test]
    fn reject_invalid_datetime() {
        assert!(parse_sqlite_datetime("not-a-time").is_none());
    }

    #[test]
    fn parse_datetime_with_t_separator_without_z() {
        assert!(parse_sqlite_datetime("2025-01-15T08:00:00").is_none());
    }

    #[cfg(feature = "server")]
    struct NoopHttp;

    #[cfg(feature = "server")]
    #[async_trait]
    impl HttpClient for NoopHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Err(crate::error::ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            Err(crate::error::ApiError::internal("not used"))
        }

        async fn post_utterances_token(
            &self,
            _body: &[u8],
            _headers: &HashMap<String, String>,
        ) -> crate::Result<UpstreamResponse> {
            Ok(UpstreamResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::new(),
            })
        }
    }

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
    async fn insert_user(db: &dyn Database, id: i64, login: &str) {
        db.execute(
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            &[
                DbValue::Integer(id),
                DbValue::Text(login.to_string()),
                DbValue::Text(format!("{}@test.com", login)),
                DbValue::Text("https://avatars/x".to_string()),
                DbValue::Text("User".to_string()),
                DbValue::Integer(0),
            ],
        )
        .await
        .expect("insert user");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn create_and_validate_session_success() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 1, "alice").await;

        let ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: &secret,
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: true,
            test_bypass_secret: None,
        };

        super::create_session(&ctx, "refresh-1", 1, 3600)
            .await
            .expect("create");
        super::validate_session(&ctx, "refresh-1", 1)
            .await
            .expect("validate");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn validate_session_rejects_wrong_user_and_revoked() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 1, "alice").await;

        let ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: &secret,
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: true,
            test_bypass_secret: None,
        };

        super::create_session(&ctx, "refresh-2", 1, 3600)
            .await
            .expect("create");

        let wrong_user = super::validate_session(&ctx, "refresh-2", 2)
            .await
            .err()
            .expect("wrong user must fail");
        assert_eq!(wrong_user.status, 401);

        super::revoke_session(&ctx, "refresh-2")
            .await
            .expect("revoke");
        let revoked = super::validate_session(&ctx, "refresh-2", 1)
            .await
            .err()
            .expect("revoked must fail");
        assert_eq!(revoked.status, 401);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn validate_session_rejects_expired_and_revoke_user_sessions() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 1, "alice").await;

        let ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: &secret,
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: true,
            test_bypass_secret: None,
        };

        super::create_session(&ctx, "refresh-3", 1, 3600)
            .await
            .expect("create");
        db.execute(
            "UPDATE sessions SET expires_at = datetime('now', '-1 seconds') WHERE refresh_token_hash = ?1",
            &[DbValue::Text(crate::auth::hash_token("refresh-3"))],
        )
        .await
        .expect("expire session");
        let expired = super::validate_session(&ctx, "refresh-3", 1)
            .await
            .err()
            .expect("expired must fail");
        assert_eq!(expired.status, 401);

        super::create_session(&ctx, "refresh-4", 1, 3600)
            .await
            .expect("create second");
        super::revoke_user_sessions(&ctx, 1)
            .await
            .expect("revoke user sessions");
        let revoked = super::validate_session(&ctx, "refresh-4", 1)
            .await
            .err()
            .expect("revoked must fail");
        assert_eq!(revoked.status, 401);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn validate_session_allows_empty_expiry_and_exercises_noop_http() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 4, "dave").await;

        let token_hash = crate::auth::hash_token("refresh-empty-exp");
        db.execute(
            "INSERT INTO sessions (refresh_token_hash, user_id, created_at, expires_at, revoked_at) \
             VALUES (?1, ?2, datetime('now'), '', NULL)",
            &[DbValue::Text(token_hash), DbValue::Integer(4)],
        )
        .await
        .expect("insert session");

        let ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: &secret,
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: true,
            test_bypass_secret: None,
        };

        super::validate_session(&ctx, "refresh-empty-exp", 4)
            .await
            .expect("empty expiry accepted");

        let gh_err = http
            .get_github_user("token")
            .await
            .err()
            .expect("noop github");
        assert_eq!(gh_err.status, 500);

        let jwks_err = http
            .get_jwks("https://example.com/jwks")
            .await
            .err()
            .expect("noop jwks");
        assert_eq!(jwks_err.status, 500);

        let utterances_ok = http
            .post_utterances_token(&[], &HashMap::new())
            .await
            .expect("noop utterances");
        assert_eq!(utterances_ok.status, 200);
    }
}
