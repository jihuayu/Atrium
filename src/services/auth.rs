use serde::Deserialize;

use crate::{
    auth::hash_token,
    db::{self, DbValue},
    error::ApiError,
    jwt,
    services::session,
    types::{AuthTokenResponse, GitHubUser, JwtClaims, NativeUser, ProviderUser},
    AppContext, Result,
};

const ACCESS_TTL_SECS: i64 = 3600;
const REFRESH_TTL_SECS: i64 = 30 * 24 * 3600;

#[derive(Debug, Deserialize)]
struct IdRow {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct UserRow {
    id: i64,
    login: String,
    email: String,
    avatar_url: String,
    r#type: String,
    site_admin: i64,
}

pub async fn resolve_or_create_user(
    ctx: &AppContext<'_>,
    provider_user: &ProviderUser,
) -> Result<GitHubUser> {
    if let Some(row) = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin \
         FROM user_identities ui \
         JOIN users u ON u.id = ui.user_id \
         WHERE ui.provider = ?1 AND ui.provider_user_id = ?2",
        &[
            DbValue::Text(provider_user.provider.clone()),
            DbValue::Text(provider_user.provider_user_id.clone()),
        ],
    )
    .await?
    {
        return Ok(to_user(row));
    }

    let mut user_id = None;
    if !provider_user.email.is_empty() && !provider_user.email.ends_with("privaterelay.appleid.com")
    {
        user_id = db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE email = ?1",
            &[DbValue::Text(provider_user.email.clone())],
        )
        .await?
        .map(|v| v.id);
    }

    let user_id = if let Some(id) = user_id {
        id
    } else {
        let login = allocate_login(ctx, &provider_user.login).await?;
        ctx.db
            .execute(
                "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
                &[
                    DbValue::Text(login.clone()),
                    DbValue::Text(provider_user.email.clone()),
                    DbValue::Text(provider_user.avatar_url.clone()),
                    DbValue::Text(provider_user.r#type.clone()),
                    DbValue::Integer(provider_user.site_admin as i64),
                ],
            )
            .await?;
        db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE login = ?1",
            &[DbValue::Text(login)],
        )
        .await?
        .ok_or_else(|| ApiError::internal("failed to create user"))?
        .id
    };

    ctx.db
        .execute(
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
             ON CONFLICT(provider, provider_user_id) DO UPDATE SET \
             user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
            &[
                DbValue::Integer(user_id),
                DbValue::Text(provider_user.provider.clone()),
                DbValue::Text(provider_user.provider_user_id.clone()),
                DbValue::Text(provider_user.email.clone()),
                DbValue::Text(provider_user.avatar_url.clone()),
            ],
        )
        .await?;

    let row = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("failed to load user"))?;

    Ok(to_user(row))
}

pub async fn issue_xtalk_jwt(ctx: &AppContext<'_>, user: &GitHubUser) -> Result<AuthTokenResponse> {
    let now = chrono::Utc::now().timestamp();
    let access_claims = JwtClaims {
        sub: user.id.to_string(),
        login: user.login.clone(),
        iss: "xtalk".to_string(),
        iat: now,
        exp: now + ACCESS_TTL_SECS,
        jti: format!("acc-{}-{}", user.id, now),
        token_type: "access".to_string(),
    };
    let refresh_claims = JwtClaims {
        sub: user.id.to_string(),
        login: user.login.clone(),
        iss: "xtalk".to_string(),
        iat: now,
        exp: now + REFRESH_TTL_SECS,
        jti: format!("ref-{}-{}", user.id, now),
        token_type: "refresh".to_string(),
    };

    let access_token = jwt::sign_jwt(&access_claims, ctx.jwt_secret)?;
    let refresh_token = jwt::sign_jwt(&refresh_claims, ctx.jwt_secret)?;

    if ctx.stateful_sessions {
        session::create_session(ctx, &refresh_token, user.id, REFRESH_TTL_SECS).await?;
    }

    Ok(AuthTokenResponse {
        access_token,
        refresh_token,
        expires_in: ACCESS_TTL_SECS,
        token_type: "Bearer".to_string(),
        user: NativeUser {
            id: user.id,
            login: user.login.clone(),
            avatar_url: user.avatar_url.clone(),
            email: user.email.clone(),
        },
    })
}

pub async fn refresh_xtalk_jwt(
    ctx: &AppContext<'_>,
    refresh_token: &str,
) -> Result<AuthTokenResponse> {
    let claims = jwt::verify_jwt(refresh_token, ctx.jwt_secret)?;
    if claims.token_type != "refresh" {
        return Err(ApiError::unauthorized());
    }

    let user_id = claims
        .sub
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized())?;

    if ctx.stateful_sessions {
        session::validate_session(ctx, refresh_token, user_id).await?;
    }

    let user = db::query_opt::<UserRow>(
        ctx.db,
        "SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?
    .map(to_user)
    .ok_or_else(ApiError::unauthorized)?;

    issue_xtalk_jwt(ctx, &user).await
}

pub async fn revoke_current_session(ctx: &AppContext<'_>, user_id: i64) -> Result<()> {
    if ctx.stateful_sessions {
        session::revoke_user_sessions(ctx, user_id).await?;
    }
    Ok(())
}

async fn allocate_login(ctx: &AppContext<'_>, preferred: &str) -> Result<String> {
    let base = if preferred.trim().is_empty() {
        "user".to_string()
    } else {
        preferred.trim().to_lowercase()
    };
    for index in 0..1000 {
        let candidate = if index == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, index)
        };
        let exists = db::query_opt::<IdRow>(
            ctx.db,
            "SELECT id FROM users WHERE login = ?1",
            &[DbValue::Text(candidate.clone())],
        )
        .await?;
        if exists.is_none() {
            return Ok(candidate);
        }
    }
    Err(ApiError::internal("unable to allocate login"))
}

fn to_user(row: UserRow) -> GitHubUser {
    GitHubUser {
        id: row.id,
        login: row.login,
        email: row.email,
        avatar_url: row.avatar_url,
        r#type: row.r#type,
        site_admin: row.site_admin != 0,
    }
}

pub async fn cache_provider_token(
    ctx: &AppContext<'_>,
    provider: &str,
    token: &str,
    user_id: i64,
    ttl_secs: i64,
) -> Result<()> {
    let token_hash = hash_token(token);
    ctx.db
        .execute(
            "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) \
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now', '+' || ?4 || ' seconds')) \
             ON CONFLICT(token_hash, provider) DO UPDATE SET \
             user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
            &[
                DbValue::Text(token_hash),
                DbValue::Text(provider.to_string()),
                DbValue::Integer(user_id),
                DbValue::Integer(ttl_secs),
            ],
        )
        .await?;
    Ok(())
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use bytes::Bytes;
    use serde::Deserialize;

    use super::{
        cache_provider_token, issue_xtalk_jwt, refresh_xtalk_jwt, resolve_or_create_user,
        revoke_current_session,
    };
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::{self, Database, DbValue},
        error::ApiError,
        types::{GitHubApiUser, GitHubUser, ProviderUser},
        AppContext,
    };

    async fn make_db() -> (tempfile::TempPath, crate::platform::server::sqlite::SqliteDatabase) {
        let db_file = tempfile::NamedTempFile::new().expect("temp file").into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = crate::platform::server::sqlite::SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, db)
    }

    struct NoopHttp;

    #[async_trait]
    impl HttpClient for NoopHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Err(ApiError::internal("not used"))
        }

        async fn get_jwks(&self, _url: &str) -> crate::Result<UpstreamResponse> {
            Err(ApiError::internal("not used"))
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

    async fn insert_user(db: &dyn Database, id: i64, login: &str, email: &str) {
        db.execute(
            "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            &[
                DbValue::Integer(id),
                DbValue::Text(login.to_string()),
                DbValue::Text(email.to_string()),
                DbValue::Text("https://avatars/x".to_string()),
                DbValue::Text("User".to_string()),
                DbValue::Integer(0),
            ],
        )
        .await
        .expect("insert user");
    }

    fn ctx<'a>(
        db: &'a dyn Database,
        http: &'a dyn HttpClient,
        secret: &'a [u8],
        stateful: bool,
    ) -> AppContext<'a> {
        AppContext {
            db,
            http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: secret,
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: stateful,
            test_bypass_secret: None,
        }
    }

    #[tokio::test]
    async fn resolve_or_create_user_creates_identity() {
        #[derive(Debug, Deserialize)]
        struct IdentityRow {
            user_id: i64,
        }

        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        let app_ctx = ctx(&db, &http, &secret, true);

        let provider_user = ProviderUser {
            provider: "google".to_string(),
            provider_user_id: "g-1".to_string(),
            login: "Alice".to_string(),
            email: "alice@example.com".to_string(),
            avatar_url: "https://avatars/a".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let user = resolve_or_create_user(&app_ctx, &provider_user)
            .await
            .expect("resolve user");
        assert_eq!(user.login, "alice");

        let identity = db::query_opt::<IdentityRow>(
            &db,
            "SELECT user_id FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
            &[
                DbValue::Text("google".to_string()),
                DbValue::Text("g-1".to_string()),
            ],
        )
        .await
        .expect("query identity")
        .expect("identity exists");
        assert_eq!(identity.user_id, user.id);
    }

    #[tokio::test]
    async fn resolve_or_create_user_reuses_identity_and_email_match() {
        #[derive(Debug, Deserialize)]
        struct CountRow {
            total: i64,
        }

        let (_db_file, db) = make_db().await;
        insert_user(&db, 9, "existing", "e@example.com").await;
        db.execute(
            "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            &[
                DbValue::Integer(9),
                DbValue::Text("google".to_string()),
                DbValue::Text("google-9".to_string()),
                DbValue::Text("e@example.com".to_string()),
                DbValue::Text("https://avatars/x".to_string()),
            ],
        )
        .await
        .expect("insert identity");

        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        let app_ctx = ctx(&db, &http, &secret, true);

        let by_identity = resolve_or_create_user(
            &app_ctx,
            &ProviderUser {
                provider: "google".to_string(),
                provider_user_id: "google-9".to_string(),
                login: "new-login".to_string(),
                email: "new@example.com".to_string(),
                avatar_url: "https://avatars/y".to_string(),
                r#type: "User".to_string(),
                site_admin: false,
            },
        )
        .await
        .expect("identity match");
        assert_eq!(by_identity.id, 9);

        let by_email = resolve_or_create_user(
            &app_ctx,
            &ProviderUser {
                provider: "apple".to_string(),
                provider_user_id: "apple-1".to_string(),
                login: "another".to_string(),
                email: "e@example.com".to_string(),
                avatar_url: "https://avatars/z".to_string(),
                r#type: "User".to_string(),
                site_admin: false,
            },
        )
        .await
        .expect("email match");
        assert_eq!(by_email.id, 9);

        let users_count = db::query_opt::<CountRow>(
            &db,
            "SELECT COUNT(*) AS total FROM users",
            &[],
        )
        .await
        .expect("count users")
        .expect("row");
        assert_eq!(users_count.total, 1);
    }

    #[tokio::test]
    async fn issue_refresh_and_revoke_flow_stateful() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 11, "alice", "alice@test.com").await;
        let app_ctx = ctx(&db, &http, &secret, true);

        let user = GitHubUser {
            id: 11,
            login: "alice".to_string(),
            email: "alice@test.com".to_string(),
            avatar_url: "https://avatars/a".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let issued = issue_xtalk_jwt(&app_ctx, &user).await.expect("issue");
        let refreshed = refresh_xtalk_jwt(&app_ctx, &issued.refresh_token)
            .await
            .expect("refresh");
        assert!(!refreshed.access_token.is_empty());

        revoke_current_session(&app_ctx, 11)
            .await
            .expect("revoke current session");
        let err = refresh_xtalk_jwt(&app_ctx, &issued.refresh_token)
            .await
            .err()
            .expect("refresh after revoke must fail");
        assert_eq!(err.status, 401);
    }

    #[tokio::test]
    async fn refresh_rejects_access_token_and_stateless_refresh_works() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 12, "bob", "bob@test.com").await;

        let user = GitHubUser {
            id: 12,
            login: "bob".to_string(),
            email: "bob@test.com".to_string(),
            avatar_url: "https://avatars/b".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let stateful_ctx = ctx(&db, &http, &secret, true);
        let issued = issue_xtalk_jwt(&stateful_ctx, &user)
            .await
            .expect("issue token");
        let access_err = refresh_xtalk_jwt(&stateful_ctx, &issued.access_token)
            .await
            .err()
            .expect("access token cannot refresh");
        assert_eq!(access_err.status, 401);

        let stateless_ctx = ctx(&db, &http, &secret, false);
        let stateless_issued = issue_xtalk_jwt(&stateless_ctx, &user)
            .await
            .expect("issue stateless");
        let refreshed = refresh_xtalk_jwt(&stateless_ctx, &stateless_issued.refresh_token)
            .await
            .expect("stateless refresh");
        assert!(!refreshed.refresh_token.is_empty());
    }

    #[tokio::test]
    async fn cache_provider_token_upsert_updates_user() {
        #[derive(Debug, Deserialize)]
        struct CacheRow {
            user_id: i64,
            provider: String,
        }

        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        insert_user(&db, 20, "u20", "u20@test.com").await;
        insert_user(&db, 21, "u21", "u21@test.com").await;

        let app_ctx = ctx(&db, &http, &secret, true);
        cache_provider_token(&app_ctx, "github", "tok-1", 20, 300)
            .await
            .expect("cache first");
        cache_provider_token(&app_ctx, "github", "tok-1", 21, 300)
            .await
            .expect("cache update");

        let row = db::query_opt::<CacheRow>(
            &db,
            "SELECT user_id, provider FROM token_cache WHERE token_hash = ?1 AND provider = ?2",
            &[
                DbValue::Text(crate::auth::hash_token("tok-1")),
                DbValue::Text("github".to_string()),
            ],
        )
        .await
        .expect("query cache")
        .expect("cache row");
        assert_eq!(row.user_id, 21);
        assert_eq!(row.provider, "github");
    }

    #[tokio::test]
    async fn resolve_or_create_user_handles_empty_login_and_exercises_noop_http() {
        let (_db_file, db) = make_db().await;
        let http = NoopHttp;
        let secret = b"test-jwt-secret-at-least-32-bytes!!".to_vec();
        let app_ctx = ctx(&db, &http, &secret, false);

        let user = resolve_or_create_user(
            &app_ctx,
            &ProviderUser {
                provider: "apple".to_string(),
                provider_user_id: "apple-empty-login".to_string(),
                login: "   ".to_string(),
                email: "x@privaterelay.appleid.com".to_string(),
                avatar_url: "https://avatars/p".to_string(),
                r#type: "User".to_string(),
                site_admin: false,
            },
        )
        .await
        .expect("create user with empty login");
        assert!(user.login.starts_with("user"));

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
