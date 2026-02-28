use crate::{
    auth::{bearer_from_header, resolve_github_user, resolve_xtalk_jwt_user},
    handlers::body_json,
    jwks,
    router::{AppRequest, AppResponse},
    services,
    types::{ProviderTokenInput, RefreshTokenInput},
    ApiError, AppContext,
};

use super::respond_native;

const TOKEN_CACHE_TTL_SECS: i64 = 3600;

pub async fn github(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(github_inner(req, ctx).await)
}

async fn github_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let input: ProviderTokenInput = body_json(&req)?;
    let user = resolve_github_user(ctx.db, ctx.http, &input.token, TOKEN_CACHE_TTL_SECS).await?;
    let tokens = services::auth::issue_xtalk_jwt(ctx, &user).await?;
    Ok(AppResponse::json(200, &tokens))
}

pub async fn google(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(google_inner(req, ctx).await)
}

async fn google_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    if ctx
        .google_client_id
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        return Ok(AppResponse::json(
            501,
            &serde_json::json!({
                "error": "not_configured",
                "message": "Google login is not enabled on this server"
            }),
        ));
    }

    let input: ProviderTokenInput = body_json(&req)?;
    let provider_user =
        jwks::verify_google_id_token(ctx.db, ctx.http, &input.token, ctx.google_client_id).await?;
    let user = services::auth::resolve_or_create_user(ctx, &provider_user).await?;
    services::auth::cache_provider_token(
        ctx,
        "google",
        &input.token,
        user.id,
        TOKEN_CACHE_TTL_SECS,
    )
    .await?;
    let tokens = services::auth::issue_xtalk_jwt(ctx, &user).await?;
    Ok(AppResponse::json(200, &tokens))
}

pub async fn apple(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(apple_inner(req, ctx).await)
}

async fn apple_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    if ctx
        .apple_app_id
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        return Ok(AppResponse::json(
            501,
            &serde_json::json!({
                "error": "not_configured",
                "message": "Apple login is not enabled on this server"
            }),
        ));
    }

    let input: ProviderTokenInput = body_json(&req)?;
    let provider_user =
        jwks::verify_apple_id_token(ctx.db, ctx.http, &input.token, ctx.apple_app_id).await?;
    let user = services::auth::resolve_or_create_user(ctx, &provider_user).await?;
    services::auth::cache_provider_token(ctx, "apple", &input.token, user.id, TOKEN_CACHE_TTL_SECS)
        .await?;
    let tokens = services::auth::issue_xtalk_jwt(ctx, &user).await?;
    Ok(AppResponse::json(200, &tokens))
}

pub async fn refresh(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(refresh_inner(req, ctx).await)
}

async fn refresh_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let refresh_token = if req.body.is_empty() {
        bearer_from_header(req.auth_header.as_deref())?.ok_or_else(ApiError::unauthorized)?
    } else {
        let input: RefreshTokenInput = body_json(&req)?;
        input.refresh_token
    };
    let tokens = services::auth::refresh_xtalk_jwt(ctx, &refresh_token).await?;
    Ok(AppResponse::json(200, &tokens))
}

pub async fn session_delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(session_delete_inner(req, ctx).await)
}

async fn session_delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let user = if let Some(user) = ctx.user {
        user.clone()
    } else {
        resolve_xtalk_jwt_user(ctx.db, req.auth_header.as_deref(), ctx.jwt_secret)
            .await?
            .ok_or_else(ApiError::unauthorized)?
    };
    services::auth::revoke_current_session(ctx, user.id).await?;
    Ok(AppResponse::no_content())
}

pub async fn me(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(me_inner(req, ctx).await)
}

async fn me_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let user = if let Some(user) = ctx.user {
        user.clone()
    } else {
        resolve_xtalk_jwt_user(ctx.db, req.auth_header.as_deref(), ctx.jwt_secret)
            .await?
            .ok_or_else(ApiError::unauthorized)?
    };

    Ok(AppResponse::json(
        200,
        &serde_json::json!({
            "id": user.id,
            "login": user.login,
            "avatar_url": user.avatar_url,
            "email": user.email,
        }),
    ))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use bytes::Bytes;

    use super::{github, me, refresh, session_delete};
    use crate::{
        auth::{HttpClient, UpstreamResponse},
        db::Database,
        error::ApiError,
        router::AppRequest,
        types::{GitHubApiUser, GitHubUser},
        AppContext,
    };

    struct MockHttp;

    #[async_trait]
    impl HttpClient for MockHttp {
        async fn get_github_user(&self, _token: &str) -> crate::Result<GitHubApiUser> {
            Ok(GitHubApiUser {
                id: 77,
                login: "gh-user".to_string(),
                email: Some("gh@test.com".to_string()),
                avatar_url: "https://avatars/gh".to_string(),
                r#type: "User".to_string(),
                site_admin: false,
            })
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

    async fn make_db() -> (tempfile::TempPath, crate::platform::server::sqlite::SqliteDatabase) {
        let db_file = tempfile::NamedTempFile::new().expect("temp file").into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = crate::platform::server::sqlite::SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, db)
    }

    fn req(path: &str, body: Bytes, auth_header: Option<String>) -> AppRequest {
        AppRequest {
            method: "POST".to_string(),
            path: path.to_string(),
            path_params: HashMap::new(),
            query: HashMap::new(),
            headers: HashMap::new(),
            auth_header,
            accept: None,
            body,
        }
    }

    fn ctx<'a>(
        db: &'a dyn Database,
        http: &'a dyn HttpClient,
        user: Option<&'a GitHubUser>,
    ) -> AppContext<'a> {
        AppContext {
            db,
            http,
            comment_cache: None,
            base_url: "http://localhost",
            user,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!",
            google_client_id: None,
            apple_app_id: None,
            stateful_sessions: false,
            test_bypass_secret: None,
        }
    }

    #[tokio::test]
    async fn github_refresh_me_and_session_delete_paths() {
        let (_db_file, db) = make_db().await;
        let http = MockHttp;

        let app_ctx = ctx(&db, &http, None);
        let github_req = req(
            "/api/v1/auth/github",
            Bytes::from_static(br#"{"token":"gh-token"}"#),
            None,
        );
        let github_resp = github(github_req, &app_ctx).await;
        assert_eq!(github_resp.status, 200);
        let github_payload: serde_json::Value =
            serde_json::from_slice(&github_resp.body).expect("github payload");
        let refresh_token = github_payload["refresh_token"]
            .as_str()
            .expect("refresh token from github")
            .to_string();

        let user = GitHubUser {
            id: 77,
            login: "gh-user".to_string(),
            email: "gh@test.com".to_string(),
            avatar_url: "https://avatars/gh".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let refresh_header_req = req(
            "/api/v1/auth/refresh",
            Bytes::new(),
            Some(format!("Bearer {}", refresh_token)),
        );
        let refresh_header_resp = refresh(refresh_header_req, &app_ctx).await;
        assert_eq!(refresh_header_resp.status, 200);

        let refresh_body_req = req(
            "/api/v1/auth/refresh",
            Bytes::from(format!(r#"{{"refresh_token":"{}"}}"#, refresh_token)),
            None,
        );
        let refresh_body_resp = refresh(refresh_body_req, &app_ctx).await;
        assert_eq!(refresh_body_resp.status, 200);

        let user_ctx = ctx(&db, &http, Some(&user));
        let me_req = req("/api/v1/auth/me", Bytes::new(), None);
        let me_resp = me(me_req, &user_ctx).await;
        assert_eq!(me_resp.status, 200);

        let delete_req = req("/api/v1/auth/session", Bytes::new(), None);
        let delete_resp = session_delete(delete_req, &user_ctx).await;
        assert_eq!(delete_resp.status, 204);
    }

    #[tokio::test]
    async fn google_apple_configured_paths_and_mock_aux_methods() {
        let (_db_file, db) = make_db().await;
        let http = MockHttp;

        let configured_ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!",
            google_client_id: Some("google-client"),
            apple_app_id: Some("apple-client"),
            stateful_sessions: false,
            test_bypass_secret: None,
        };

        let google_req = req(
            "/api/v1/auth/google",
            Bytes::from_static(br#"{"token":"bad-google-token"}"#),
            None,
        );
        let google_resp = super::google(google_req, &configured_ctx).await;
        assert_eq!(google_resp.status, 401);

        let apple_req = req(
            "/api/v1/auth/apple",
            Bytes::from_static(br#"{"token":"bad-apple-token"}"#),
            None,
        );
        let apple_resp = super::apple(apple_req, &configured_ctx).await;
        assert_eq!(apple_resp.status, 401);

        let jwks_err = http
            .get_jwks("https://example.com/jwks")
            .await
            .err()
            .expect("not used");
        assert_eq!(jwks_err.status, 500);

        let utterances_ok = http
            .post_utterances_token(&[], &HashMap::new())
            .await
            .expect("ok");
        assert_eq!(utterances_ok.status, 200);
    }
}
