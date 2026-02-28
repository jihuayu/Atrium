pub mod d1;
pub mod http;

use crate::{
    auth::{bearer_from_header, resolve_github_user, resolve_xtalk_jwt_user},
    router::{parse_query_string, AppRequest, AppResponse, AppRouter},
    AppContext,
};
use worker::{event, Context, Env, Method, Request, Response, Result};

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
    pub fn from_env(env: &worker::Env) -> Self {
        let base_url = env
            .var("BASE_URL")
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
        let token_cache_ttl = env
            .var("TOKEN_CACHE_TTL")
            .ok()
            .and_then(|v| v.to_string().parse::<i64>().ok())
            .unwrap_or(3600);
        let jwt_secret = env
            .secret("JWT_SECRET")
            .map(|v| parse_secret_bytes(&v.to_string()))
            .or_else(|_| {
                env.var("JWT_SECRET")
                    .map(|v| parse_secret_bytes(&v.to_string()))
            })
            .unwrap_or_else(|_| b"xtalk-dev-secret-change-me".to_vec());
        let google_client_id = env
            .var("GOOGLE_CLIENT_ID")
            .ok()
            .map(|v| v.to_string())
            .filter(|v| !v.trim().is_empty());
        let apple_app_id = env
            .var("APPLE_APP_ID")
            .ok()
            .map(|v| v.to_string())
            .filter(|v| !v.trim().is_empty());
        let test_bypass_secret = env
            .var("XTALK_TEST_BYPASS_SECRET")
            .ok()
            .map(|v| v.to_string())
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
}

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

async fn dispatch(
    req: Request,
    env: &Env,
    state: &WorkerState,
    router: &AppRouter,
) -> crate::Result<AppResponse> {
    let app_req = to_app_request(req).await?;

    let db = D1Db {
        db: env
            .d1("DB")
            .map_err(|e| crate::ApiError::internal(format!("missing D1 binding DB: {}", e)))?,
    };
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
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
        .or_else(|_| {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, value)
        })
        .unwrap_or_else(|_| value.as_bytes().to_vec())
}
