pub mod cache;
pub mod http;
pub mod sqlite;

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, Request as HttpRequest, StatusCode},
    response::Response,
    Router,
};
use tower_http::cors::{Any, CorsLayer};

use crate::{
    auth::{bearer_from_header, resolve_github_user, resolve_xtalk_jwt_user},
    router::{parse_query_string, AppRequest, AppResponse, AppRouter},
    types::GitHubUser,
    ApiError, AppContext, Result,
};

use self::{cache::CommentCache, http::ReqwestHttpClient, sqlite::SqliteDatabase};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<SqliteDatabase>,
    pub http: Arc<ReqwestHttpClient>,
    pub cache: Arc<CommentCache>,
    pub router: Arc<AppRouter>,
    pub base_url: String,
    pub token_cache_ttl: i64,
    pub jwt_secret: Vec<u8>,
    pub google_client_id: Option<String>,
    pub apple_app_id: Option<String>,
    pub test_bypass_secret: Option<String>,
}

pub async fn build_app(
    database_url: &str,
    base_url: String,
    token_cache_ttl: i64,
    cache_max_issues: u64,
    cache_ttl_secs: u64,
    jwt_secret: Vec<u8>,
    google_client_id: Option<String>,
    apple_app_id: Option<String>,
) -> Result<Router> {
    let db = Arc::new(SqliteDatabase::connect_and_migrate(database_url).await?);
    let http = Arc::new(ReqwestHttpClient::new()?);
    let cache = Arc::new(CommentCache::new(cache_max_issues, cache_ttl_secs));

    let state = AppState {
        db,
        http,
        cache,
        router: Arc::new(AppRouter::new()),
        base_url,
        token_cache_ttl,
        jwt_secret,
        google_client_id,
        apple_app_id,
        test_bypass_secret: std::env::var("ATRIUM_TEST_BYPASS_SECRET")
            .or_else(|_| std::env::var("XTALK_TEST_BYPASS_SECRET"))
            .ok()
            .filter(|v| !v.trim().is_empty()),
    };

    let app = Router::new().fallback(dispatch).with_state(state).layer(
        CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(Any),
    );

    Ok(app)
}

async fn dispatch(State(state): State<AppState>, req: HttpRequest<Body>) -> Response {
    match dispatch_inner(state, req).await {
        Ok(response) => response,
        Err(error) => to_axum_response(AppResponse::from_error(error)),
    }
}

async fn dispatch_inner(state: AppState, req: HttpRequest<Body>) -> Result<Response> {
    let (parts, body) = req.into_parts();
    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_ascii_lowercase(), v.to_string()))
        })
        .collect();
    let body = to_bytes(body, 8 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::internal(format!("read request body failed: {}", e)))?;

    let auth_header = header_value(&parts.headers, header::AUTHORIZATION);
    let accept = header_value(&parts.headers, header::ACCEPT);

    let app_req = AppRequest {
        method: parts.method.as_str().to_string(),
        path: parts.uri.path().to_string(),
        path_params: std::collections::HashMap::new(),
        query: parse_query_string(parts.uri.query()),
        headers,
        auth_header,
        accept,
        body,
    };

    let user = resolve_request_user(&app_req.path, app_req.auth_header.as_deref(), &state).await?;
    let ctx = AppContext {
        db: state.db.as_ref(),
        http: state.http.as_ref(),
        comment_cache: Some(state.cache.as_ref()),
        base_url: &state.base_url,
        user: user.as_ref(),
        jwt_secret: &state.jwt_secret,
        google_client_id: state.google_client_id.as_deref(),
        apple_app_id: state.apple_app_id.as_deref(),
        stateful_sessions: true,
        test_bypass_secret: state.test_bypass_secret.as_deref(),
    };

    let app_response = state.router.handle(app_req, &ctx).await;
    Ok(to_axum_response(app_response))
}

async fn resolve_request_user(
    path: &str,
    auth_header: Option<&str>,
    state: &AppState,
) -> Result<Option<GitHubUser>> {
    #[cfg(any(feature = "test-utils", feature = "worker"))]
    if let Some(header) = auth_header {
        if let Some(user) =
            crate::auth::try_test_bypass(header, state.test_bypass_secret.as_deref())
        {
            crate::auth::upsert_auth_user(state.db.as_ref(), &user).await?;
            return Ok(Some(user));
        }
    }

    if path.starts_with("/api/v1/auth/") {
        return Ok(None);
    }

    if path.starts_with("/api/v1/") {
        return resolve_xtalk_jwt_user(state.db.as_ref(), auth_header, &state.jwt_secret).await;
    }

    let token = bearer_from_header(auth_header)?;
    match token {
        None => Ok(None),
        Some(token) => {
            let user = resolve_github_user(
                state.db.as_ref(),
                state.http.as_ref(),
                &token,
                state.token_cache_ttl,
            )
            .await?;
            Ok(Some(user))
        }
    }
}

fn to_axum_response(app: AppResponse) -> Response {
    let mut response = Response::new(Body::from(app.body));
    *response.status_mut() =
        StatusCode::from_u16(app.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    for (name, value) in app.headers {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(header_name, header_value);
        }
    }

    response
}

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::Body,
        extract::State,
        http::Request as HttpRequest,
    };

    use super::{
        dispatch, resolve_request_user, AppRouter, AppState, CommentCache, ReqwestHttpClient,
        SqliteDatabase,
    };
    use crate::{auth::hash_token, db::{Database, DbValue}};

    async fn make_db() -> (tempfile::TempPath, Arc<SqliteDatabase>) {
        let db_file = tempfile::NamedTempFile::new().expect("temp file").into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("db init");
        (db_file, Arc::new(db))
    }

    async fn make_state() -> (tempfile::TempPath, AppState) {
        let (db_file, db) = make_db().await;
        let http = ReqwestHttpClient::new().expect("http");
        let state = AppState {
            db,
            http: Arc::new(http),
            cache: Arc::new(CommentCache::new(100, 60)),
            router: Arc::new(AppRouter::new()),
            base_url: "http://localhost".to_string(),
            token_cache_ttl: 3600,
            jwt_secret: b"test-jwt-secret-at-least-32-bytes!!".to_vec(),
            google_client_id: None,
            apple_app_id: None,
            test_bypass_secret: None,
        };
        (db_file, state)
    }

    #[tokio::test]
    async fn resolve_request_user_supports_github_bearer() {
        let (_db_file, state) = make_state().await;
        state
            .db
            .execute(
                "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES \
                 (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
                &[
                    DbValue::Integer(42),
                    DbValue::Text("alice".to_string()),
                    DbValue::Text("alice@test.com".to_string()),
                    DbValue::Text("https://avatars/a".to_string()),
                    DbValue::Text("User".to_string()),
                    DbValue::Integer(0),
                ],
            )
            .await
            .expect("insert user");
        state
            .db
            .execute(
                "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) VALUES (?1, 'github', ?2, datetime('now'), datetime('now', '+3600 seconds'))",
                &[
                    DbValue::Text(hash_token("gh-token")),
                    DbValue::Integer(42),
                ],
            )
            .await
            .expect("insert token cache");

        let user = resolve_request_user("/repos/o/r/issues", Some("Bearer gh-token"), &state)
            .await
            .expect("resolve user")
            .expect("has user");
        assert_eq!(user.login, "alice");
    }

    #[tokio::test]
    async fn dispatch_turns_errors_into_http_response() {
        let (_db_file, state) = make_state().await;
        let req = HttpRequest::builder()
            .method("GET")
            .uri("/repos/o/r/issues")
            .header("Authorization", "Basic invalid")
            .body(Body::empty())
            .expect("request");

        let resp = dispatch(State(state), req).await;
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}
