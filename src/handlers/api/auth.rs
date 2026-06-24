use crate::{
    auth::{bearer_from_header, resolve_github_user, resolve_xtalk_jwt_user},
    cookies,
    handlers::{body_json, query_value},
    jwks,
    router::{AppRequest, AppResponse},
    services,
    types::{AuthTokenResponse, ProviderTokenInput, RefreshTokenInput},
    ApiError, AppContext,
};

use super::respond_native;

const TOKEN_CACHE_TTL_SECS: i64 = 3600;
const OAUTH_STATE_MAX_AGE_SECS: i64 = 600;

pub async fn github_authorize(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(github_authorize_inner(req, ctx).await)
}

async fn github_authorize_inner(
    req: AppRequest,
    ctx: &AppContext<'_>,
) -> crate::Result<AppResponse> {
    let client_id = ctx
        .github_client_id
        .ok_or_else(|| ApiError::new(501, "GitHub OAuth is not enabled on this server"))?;

    let redirect_uri = query_value(&req, "redirect_uri")
        .ok_or_else(|| ApiError::bad_request("missing redirect_uri query parameter"))?;
    let state = query_value(&req, "state").unwrap_or_default();
    let state_token = build_oauth_state(ctx, &redirect_uri, &state);

    let github_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&state={}&scope=user:email",
        urlencoding::encode(client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&state_token),
    );

    Ok(AppResponse::redirect(&github_url))
}

pub async fn github_callback(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(github_callback_inner(req, ctx).await)
}

async fn github_callback_inner(
    req: AppRequest,
    ctx: &AppContext<'_>,
) -> crate::Result<AppResponse> {
    let client_id = ctx
        .github_client_id
        .ok_or_else(|| ApiError::new(501, "GitHub OAuth is not enabled on this server"))?;
    let client_secret = ctx
        .github_client_secret
        .ok_or_else(|| ApiError::internal("GitHub OAuth client secret is not configured"))?;

    let code = query_value(&req, "code")
        .ok_or_else(|| ApiError::bad_request("missing code query parameter"))?;
    let state_token = query_value(&req, "state")
        .ok_or_else(|| ApiError::bad_request("missing state query parameter"))?;

    let redirect_uri = verify_oauth_state(ctx, &state_token)?;

    let gh_token = ctx
        .http
        .exchange_github_oauth_code(&code, client_id, client_secret, &redirect_uri)
        .await?;

    let user = resolve_github_user(ctx.db, ctx.http, &gh_token, TOKEN_CACHE_TTL_SECS).await?;
    let tokens = services::auth::issue_xtalk_jwt(ctx, &user).await?;

    let secure = cookies::secure_from_base_url(ctx.base_url);
    let response = AppResponse::redirect(&redirect_uri)
        .with_header(
            "Set-Cookie",
            &cookies::build_set_cookie(
                cookies::ACCESS_COOKIE,
                &tokens.access_token,
                tokens.expires_in,
                secure,
            ),
        )
        .with_header(
            "Set-Cookie",
            &cookies::build_set_cookie(
                cookies::REFRESH_COOKIE,
                &tokens.refresh_token,
                30 * 24 * 3600,
                secure,
            ),
        );
    Ok(response)
}

/// Build a signed OAuth state token encoding the redirect_uri and a timestamp.
fn build_oauth_state(ctx: &AppContext<'_>, redirect_uri: &str, user_state: &str) -> String {
    use base64::{engine::general_purpose, Engine};
    let now = chrono::Utc::now().timestamp();
    let payload = format!("{}|{}|{}", now, redirect_uri, user_state);
    let signature = hmac_sha256_hex(ctx.jwt_secret, &payload);
    let combined = format!("{}\n{}", payload, signature);
    general_purpose::URL_SAFE_NO_PAD.encode(combined)
}

/// Verify an OAuth state token and return the redirect_uri.
fn verify_oauth_state(ctx: &AppContext<'_>, state_token: &str) -> crate::Result<String> {
    use base64::{engine::general_purpose, Engine};
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(state_token)
        .map_err(|_| ApiError::bad_request("invalid OAuth state"))?;
    let combined = String::from_utf8(decoded)
        .map_err(|_| ApiError::bad_request("invalid OAuth state"))?;
    let (payload, signature) = combined
        .split_once('\n')
        .ok_or_else(|| ApiError::bad_request("invalid OAuth state format"))?;
    let expected_sig = hmac_sha256_hex(ctx.jwt_secret, payload);
    if signature != expected_sig {
        return Err(ApiError::bad_request("OAuth state signature mismatch"));
    }
    let parts: Vec<&str> = payload.splitn(3, '|').collect();
    if parts.len() < 2 {
        return Err(ApiError::bad_request("invalid OAuth state payload"));
    }
    let timestamp: i64 = parts[0]
        .parse()
        .map_err(|_| ApiError::bad_request("invalid OAuth state timestamp"))?;
    let now = chrono::Utc::now().timestamp();
    if now - timestamp > OAUTH_STATE_MAX_AGE_SECS {
        return Err(ApiError::bad_request("OAuth state expired"));
    }
    Ok(parts[1].to_string())
}

fn hmac_sha256_hex(key: &[u8], message: &str) -> String {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub async fn github(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(github_inner(req, ctx).await)
}

async fn github_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let input: ProviderTokenInput = body_json(&req)?;
    let user = resolve_github_user(ctx.db, ctx.http, &input.token, TOKEN_CACHE_TTL_SECS).await?;
    let tokens = services::auth::issue_xtalk_jwt(ctx, &user).await?;
    Ok(with_auth_cookies(ctx, AppResponse::json(200, &tokens), &tokens))
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
    Ok(with_auth_cookies(ctx, AppResponse::json(200, &tokens), &tokens))
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
    Ok(with_auth_cookies(ctx, AppResponse::json(200, &tokens), &tokens))
}

pub async fn refresh(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(refresh_inner(req, ctx).await)
}

async fn refresh_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let refresh_token = if !req.body.is_empty() {
        let input: RefreshTokenInput = body_json(&req)?;
        input.refresh_token
    } else if let Some(header) = req.auth_header.as_deref() {
        bearer_from_header(Some(header))?.ok_or_else(ApiError::unauthorized)?
    } else if let Some(cookie_header) = req.headers.get("cookie") {
        cookies::cookie_value(cookie_header, cookies::REFRESH_COOKIE)
            .map(|v| v.to_string())
            .ok_or_else(ApiError::unauthorized)?
    } else {
        return Err(ApiError::unauthorized());
    };
    let tokens = services::auth::refresh_xtalk_jwt(ctx, &refresh_token).await?;
    Ok(with_auth_cookies(ctx, AppResponse::json(200, &tokens), &tokens))
}

pub async fn session_delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(session_delete_inner(req, ctx).await)
}

async fn session_delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let user = if let Some(user) = ctx.user {
        user.clone()
    } else if let Some(auth_header) = req.auth_header.as_deref() {
        resolve_xtalk_jwt_user(ctx.db, Some(auth_header), ctx.jwt_secret)
            .await?
            .ok_or_else(ApiError::unauthorized)?
    } else if let Some(cookie_header) = req.headers.get("cookie") {
        let access = cookies::cookie_value(cookie_header, cookies::ACCESS_COOKIE)
            .ok_or_else(ApiError::unauthorized)?;
        resolve_xtalk_jwt_user(ctx.db, Some(&format!("Bearer {}", access)), ctx.jwt_secret)
            .await?
            .ok_or_else(ApiError::unauthorized)?
    } else {
        return Err(ApiError::unauthorized());
    };
    services::auth::revoke_current_session(ctx, user.id).await?;
    let secure = cookies::secure_from_base_url(ctx.base_url);
    Ok(AppResponse::no_content()
        .with_header("Set-Cookie", &cookies::clear_cookie(cookies::ACCESS_COOKIE, secure))
        .with_header("Set-Cookie", &cookies::clear_cookie(cookies::REFRESH_COOKIE, secure)))
}

/// Attach `Set-Cookie` headers for the access and refresh tokens to a response.
fn with_auth_cookies(
    ctx: &AppContext<'_>,
    mut response: AppResponse,
    tokens: &AuthTokenResponse,
) -> AppResponse {
    let secure = cookies::secure_from_base_url(ctx.base_url);
    response = response.with_header(
        "Set-Cookie",
        &cookies::build_set_cookie(cookies::ACCESS_COOKIE, &tokens.access_token, tokens.expires_in, secure),
    );
    response = response.with_header(
        "Set-Cookie",
        &cookies::build_set_cookie(
            cookies::REFRESH_COOKIE,
            &tokens.refresh_token,
            30 * 24 * 3600,
            secure,
        ),
    );
    response
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

    use super::{
        build_oauth_state, github, github_authorize, me, refresh, session_delete,
        verify_oauth_state,
    };
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
            github_client_id: None,
            github_client_secret: None,
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
    async fn oauth_state_round_trip_and_tamper_detection() {
        let (_db_file, db) = make_db().await;
        let http = MockHttp;
        let app_ctx = ctx(&db, &http, None);

        // Round trip: build → verify returns the same redirect_uri.
        let state = build_oauth_state(&app_ctx, "https://app.example/cb", "user123");
        let redirect = verify_oauth_state(&app_ctx, &state).expect("verify");
        assert_eq!(redirect, "https://app.example/cb");

        // Tampered signature → error.
        let mut bad = state.clone();
        bad.pop();
        bad.push('X');
        assert!(verify_oauth_state(&app_ctx, &bad).is_err());

        // Garbage input → error.
        assert!(verify_oauth_state(&app_ctx, "not-a-valid-token").is_err());
    }

    #[tokio::test]
    async fn github_authorize_returns_501_when_not_configured() {
        let (_db_file, db) = make_db().await;
        let http = MockHttp;
        let app_ctx = ctx(&db, &http, None);

        let mut q = HashMap::new();
        q.insert("redirect_uri".to_string(), "https://app.example/cb".to_string());
        let req = AppRequest {
            method: "GET".to_string(),
            path: "/api/v1/auth/github/authorize".to_string(),
            path_params: HashMap::new(),
            query: q,
            headers: HashMap::new(),
            auth_header: None,
            accept: None,
            body: Bytes::new(),
        };
        let resp = github_authorize(req, &app_ctx).await;
        assert_eq!(resp.status, 501);
    }

    #[tokio::test]
    async fn github_authorize_redirects_to_github_when_configured() {
        let (_db_file, db) = make_db().await;
        let http = MockHttp;
        let app_ctx = AppContext {
            db: &db,
            http: &http,
            comment_cache: None,
            base_url: "http://localhost",
            user: None,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!",
            google_client_id: None,
            apple_app_id: None,
            github_client_id: Some("gh-client-id"),
            github_client_secret: Some("gh-secret"),
            stateful_sessions: false,
            test_bypass_secret: None,
        };

        let mut q = HashMap::new();
        q.insert("redirect_uri".to_string(), "https://app.example/cb".to_string());
        let req = AppRequest {
            method: "GET".to_string(),
            path: "/api/v1/auth/github/authorize".to_string(),
            path_params: HashMap::new(),
            query: q,
            headers: HashMap::new(),
            auth_header: None,
            accept: None,
            body: Bytes::new(),
        };
        let resp = github_authorize(req, &app_ctx).await;
        assert_eq!(resp.status, 302);
        let location = resp
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("location"))
            .map(|(_, v)| v.clone())
            .expect("location header");
        assert!(location.starts_with("https://github.com/login/oauth/authorize"));
        assert!(location.contains("client_id=gh-client-id"));
    }

    #[tokio::test]
    async fn github_login_sets_auth_cookies_and_refresh_reads_cookie() {
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

        // Set-Cookie headers must be present for both access and refresh.
        let set_cookies: Vec<&String> = github_resp
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v)
            .collect();
        assert_eq!(set_cookies.len(), 2);
        let access_cookie = set_cookies
            .iter()
            .find(|v| v.starts_with("atrium_access="))
            .expect("access cookie");
        let refresh_cookie = set_cookies
            .iter()
            .find(|v| v.starts_with("atrium_refresh="))
            .expect("refresh cookie");
        assert!(access_cookie.contains("HttpOnly"));
        assert!(access_cookie.contains("SameSite=Lax"));
        // base_url is http://localhost so Secure must NOT be present.
        assert!(!access_cookie.contains("Secure"));

        // Extract the refresh token value from the cookie to use it via cookie.
        let refresh_value = refresh_cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("atrium_refresh=");

        // Refresh via cookie header (no body, no Authorization).
        let mut headers = HashMap::new();
        headers.insert(
            "cookie".to_string(),
            format!("atrium_refresh={}", refresh_value),
        );
        let cookie_req = AppRequest {
            method: "POST".to_string(),
            path: "/api/v1/auth/refresh".to_string(),
            path_params: HashMap::new(),
            query: HashMap::new(),
            headers,
            auth_header: None,
            accept: None,
            body: Bytes::new(),
        };
        let refresh_resp = refresh(cookie_req, &app_ctx).await;
        assert_eq!(refresh_resp.status, 200);
        let refresh_cookies: Vec<&String> = refresh_resp
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v)
            .collect();
        assert_eq!(refresh_cookies.len(), 2);
    }

    #[tokio::test]
    async fn session_delete_clears_cookies() {
        let (_db_file, db) = make_db().await;
        let http = MockHttp;

        let user = GitHubUser {
            id: 77,
            login: "gh-user".to_string(),
            email: "gh@test.com".to_string(),
            avatar_url: "https://avatars/gh".to_string(),
            r#type: "User".to_string(),
            site_admin: false,
        };

        let user_ctx = ctx(&db, &http, Some(&user));
        let delete_req = req("/api/v1/auth/session", Bytes::new(), None);
        let delete_resp = session_delete(delete_req, &user_ctx).await;
        assert_eq!(delete_resp.status, 204);
        let clear_cookies: Vec<&String> = delete_resp
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v)
            .collect();
        assert_eq!(clear_cookies.len(), 2);
        assert!(clear_cookies.iter().any(|v| v.starts_with("atrium_access=") && v.contains("Max-Age=0")));
        assert!(clear_cookies.iter().any(|v| v.starts_with("atrium_refresh=") && v.contains("Max-Age=0")));
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
            github_client_id: None,
            github_client_secret: None,
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
