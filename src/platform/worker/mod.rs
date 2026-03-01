pub mod d1;
pub mod http;

use base64::Engine;

#[cfg(target_arch = "wasm32")]
use crate::{
    AppContext,
    auth::{bearer_from_header, resolve_github_user, resolve_xtalk_jwt_user},
    router::{AppRequest, AppResponse, AppRouter, parse_query_string},
};
#[cfg(target_arch = "wasm32")]
use worker::{Context, Env, Method, Request, Response, Result, event};

#[cfg(target_arch = "wasm32")]
use self::{d1::D1Db, http::WorkerHttpClient};

pub struct WorkerState {
    pub base_url: String,
    pub token_cache_ttl: i64,
    pub jwt_secret: Vec<u8>,
    pub google_client_id: Option<String>,
    pub apple_app_id: Option<String>,
    pub test_bypass_secret: Option<String>,
}

impl WorkerState {
    fn from_lookup(
        mut var_lookup: impl FnMut(&str) -> Option<String>,
        mut secret_lookup: impl FnMut(&str) -> Option<String>,
    ) -> Self {
        let base_url =
            var_lookup("BASE_URL").unwrap_or_else(|| "http://127.0.0.1:8787".to_string());
        let token_cache_ttl = var_lookup("TOKEN_CACHE_TTL")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(3600);
        let jwt_secret = secret_lookup("JWT_SECRET")
            .or_else(|| var_lookup("JWT_SECRET"))
            .map(|v| parse_secret_bytes(&v))
            .unwrap_or_default();
        let google_client_id = var_lookup("GOOGLE_CLIENT_ID").filter(|v| !v.trim().is_empty());
        let apple_app_id = var_lookup("APPLE_APP_ID").filter(|v| !v.trim().is_empty());
        let test_bypass_secret = var_lookup("ATRIUM_TEST_BYPASS_SECRET")
            .or_else(|| var_lookup("XTALK_TEST_BYPASS_SECRET"))
            .filter(|v| !v.trim().is_empty());

        Self {
            base_url,
            token_cache_ttl,
            jwt_secret,
            google_client_id,
            apple_app_id,
            test_bypass_secret,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn from_env(env: &worker::Env) -> Self {
        Self::from_lookup(
            |key| env.var(key).ok().map(|v| v.to_string()),
            |key| env.secret(key).ok().map(|v| v.to_string()),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env_for_tests(
        var_lookup: impl FnMut(&str) -> Option<String>,
        secret_lookup: impl FnMut(&str) -> Option<String>,
    ) -> Self {
        Self::from_lookup(var_lookup, secret_lookup)
    }
}

#[cfg(target_arch = "wasm32")]
#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() == Method::Options {
        return add_cors(Response::empty()?.with_status(204));
    }

    let state = WorkerState::from_env(&env);
    let router = AppRouter::new();

    let app_response = match dispatch(req, &env, &state, &router).await {
        Ok(response) => response,
        Err(error) => AppResponse::from_error(error),
    };

    let response = to_worker_response(app_response)?;
    add_cors(response)
}

#[cfg(target_arch = "wasm32")]
async fn dispatch(
    req: Request,
    env: &Env,
    state: &WorkerState,
    router: &AppRouter,
) -> crate::Result<AppResponse> {
    let app_req = to_app_request(req).await?;

    if app_req.path.starts_with("/api/v1/") && state.jwt_secret.len() < 16 {
        return Err(crate::ApiError::internal("JWT_SECRET is not configured"));
    }

    let db = D1Db::from_database(
        env.d1("DB")
            .map_err(|e| crate::ApiError::internal(format!("missing D1 binding DB: {}", e)))?,
    )?;
    let http = WorkerHttpClient;

    if let Some(header) = app_req.auth_header.as_deref() {
        if let Some(user) =
            crate::auth::try_test_bypass(header, state.test_bypass_secret.as_deref())
        {
            crate::auth::upsert_auth_user(&db, &user).await?;
            let ctx = AppContext {
                db: &db,
                http: &http,
                comment_cache: None,
                base_url: &state.base_url,
                user: Some(&user),
                jwt_secret: &state.jwt_secret,
                google_client_id: state.google_client_id.as_deref(),
                apple_app_id: state.apple_app_id.as_deref(),
                stateful_sessions: false,
                test_bypass_secret: state.test_bypass_secret.as_deref(),
            };
            return Ok(router.handle(app_req, &ctx).await);
        }
    }

    let user = if app_req.path.starts_with("/api/v1/auth/") {
        None
    } else if app_req.path.starts_with("/api/v1/") {
        resolve_xtalk_jwt_user(&db, app_req.auth_header.as_deref(), &state.jwt_secret).await?
    } else {
        let token = bearer_from_header(app_req.auth_header.as_deref())?;
        match token {
            None => None,
            Some(token) => {
                Some(resolve_github_user(&db, &http, &token, state.token_cache_ttl).await?)
            }
        }
    };

    let ctx = AppContext {
        db: &db,
        http: &http,
        comment_cache: None,
        base_url: &state.base_url,
        user: user.as_ref(),
        jwt_secret: &state.jwt_secret,
        google_client_id: state.google_client_id.as_deref(),
        apple_app_id: state.apple_app_id.as_deref(),
        stateful_sessions: false,
        test_bypass_secret: state.test_bypass_secret.as_deref(),
    };

    Ok(router.handle(app_req, &ctx).await)
}

#[cfg(target_arch = "wasm32")]
async fn to_app_request(req: Request) -> crate::Result<AppRequest> {
    let mut req = req;

    let method = match req.method() {
        Method::Get => "GET".to_string(),
        Method::Post => "POST".to_string(),
        Method::Patch => "PATCH".to_string(),
        Method::Delete => "DELETE".to_string(),
        Method::Options => "OPTIONS".to_string(),
        Method::Put => "PUT".to_string(),
        Method::Head => "HEAD".to_string(),
        other => other.to_string(),
    };

    let path = req.path();
    let query = parse_query_string(
        req.url()
            .map_err(|e| crate::ApiError::internal(format!("parse request url failed: {}", e)))?
            .query(),
    );
    let auth_header = req.headers().get("Authorization").ok().flatten();
    let accept = req.headers().get("Accept").ok().flatten();
    let body = req
        .bytes()
        .await
        .map_err(|e| crate::ApiError::internal(format!("read request body failed: {}", e)))?;
    let headers = req
        .headers()
        .entries()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();

    Ok(AppRequest {
        method,
        path,
        path_params: std::collections::HashMap::new(),
        query,
        headers,
        auth_header,
        accept,
        body: bytes::Bytes::from(body),
    })
}

#[cfg(target_arch = "wasm32")]
fn to_worker_response(app: AppResponse) -> Result<Response> {
    let mut response = if app.body.is_empty() {
        Response::empty()?.with_status(app.status)
    } else {
        let payload = String::from_utf8_lossy(&app.body).to_string();
        Response::ok(&payload)?.with_status(app.status)
    };

    for (name, value) in app.headers {
        let _ = response.headers_mut().set(&name, &value);
    }

    Ok(response)
}

#[cfg(target_arch = "wasm32")]
fn add_cors(mut response: Response) -> Result<Response> {
    let h = response.headers_mut();
    h.set("Access-Control-Allow-Origin", "*")?;
    h.set(
        "Access-Control-Allow-Methods",
        "GET, POST, PATCH, DELETE, OPTIONS",
    )?;
    h.set(
        "Access-Control-Allow-Headers",
        "Authorization, Content-Type, Accept",
    )?;
    h.set("Access-Control-Expose-Headers", "Link")?;
    Ok(response)
}

fn parse_secret_bytes(value: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value))
        .unwrap_or_else(|_| value.as_bytes().to_vec())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::HashMap;

    use super::{WorkerState, parse_secret_bytes};

    #[test]
    fn parse_secret_bytes_supports_standard_urlsafe_and_plain() {
        assert_eq!(parse_secret_bytes("YXRyaXVt"), b"atrium".to_vec());
        assert_eq!(
            parse_secret_bytes("YXRyaXVtLXNlY3JldA"),
            b"atrium-secret".to_vec()
        );
        assert_eq!(parse_secret_bytes("not@base64"), b"not@base64".to_vec());
    }

    #[test]
    fn worker_state_from_lookup_applies_defaults_and_fallbacks() {
        let vars: HashMap<String, String> = HashMap::new();
        let state =
            WorkerState::from_env_for_tests(|k| vars.get(k).cloned(), |_k| Option::<String>::None);

        assert_eq!(state.base_url, "http://127.0.0.1:8787");
        assert_eq!(state.token_cache_ttl, 3600);
        assert!(state.jwt_secret.is_empty());
        assert_eq!(state.google_client_id, None);
        assert_eq!(state.apple_app_id, None);
        assert_eq!(state.test_bypass_secret, None);
    }

    #[test]
    fn worker_state_from_lookup_prefers_secret_and_filters_empty_values() {
        let mut vars = HashMap::new();
        vars.insert("BASE_URL".to_string(), "https://atrium.example".to_string());
        vars.insert("TOKEN_CACHE_TTL".to_string(), "abc".to_string());
        vars.insert("JWT_SECRET".to_string(), "YXRyaXVtLXZhcg".to_string());
        vars.insert("GOOGLE_CLIENT_ID".to_string(), "   ".to_string());
        vars.insert("APPLE_APP_ID".to_string(), "apple-client".to_string());
        vars.insert(
            "XTALK_TEST_BYPASS_SECRET".to_string(),
            "legacy-bypass".to_string(),
        );

        let mut secrets = HashMap::new();
        secrets.insert("JWT_SECRET".to_string(), "YXRyaXVtLXNlY3JldA".to_string());

        let state =
            WorkerState::from_env_for_tests(|k| vars.get(k).cloned(), |k| secrets.get(k).cloned());

        assert_eq!(state.base_url, "https://atrium.example");
        assert_eq!(state.token_cache_ttl, 3600);
        assert_eq!(state.jwt_secret, b"atrium-secret".to_vec());
        assert_eq!(state.google_client_id, None);
        assert_eq!(state.apple_app_id.as_deref(), Some("apple-client"));
        assert_eq!(state.test_bypass_secret.as_deref(), Some("legacy-bypass"));
    }
}
