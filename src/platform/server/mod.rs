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
    auth::{bearer_from_header, resolve_user},
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
}

pub async fn build_app(
    database_url: &str,
    base_url: String,
    token_cache_ttl: i64,
    cache_max_issues: u64,
    cache_ttl_secs: u64,
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

    let user = resolve_request_user(app_req.auth_header.as_deref(), &state).await?;
    let ctx = AppContext {
        db: state.db.as_ref(),
        http: state.http.as_ref(),
        comment_cache: Some(state.cache.as_ref()),
        base_url: &state.base_url,
        user: user.as_ref(),
    };

    let app_response = state.router.handle(app_req, &ctx).await;
    Ok(to_axum_response(app_response))
}

async fn resolve_request_user(
    auth_header: Option<&str>,
    state: &AppState,
) -> Result<Option<GitHubUser>> {
    let token = bearer_from_header(auth_header)?;
    match token {
        None => Ok(None),
        Some(token) => {
            let user = resolve_user(
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
