pub mod api;
pub mod comments;
pub mod exports;
pub mod issues;
pub mod labels;
pub mod reactions;
pub mod search;
pub mod utterances;

use serde::de::DeserializeOwned;

use crate::{
    fmt::user::to_api_user,
    markdown,
    router::{AppRequest, AppResponse},
    types::RenderMarkdownInput,
    ApiError, AppContext,
};

pub fn path_param(req: &AppRequest, name: &str) -> crate::Result<String> {
    req.path_params
        .get(name)
        .cloned()
        .ok_or_else(|| ApiError::bad_request(format!("missing route param: {}", name)))
}

pub fn path_i64(req: &AppRequest, name: &str) -> crate::Result<i64> {
    let raw = path_param(req, name)?;
    raw.parse::<i64>()
        .map_err(|_| ApiError::bad_request(format!("invalid integer param: {}", name)))
}

pub fn query_value(req: &AppRequest, key: &str) -> Option<String> {
    req.query.get(key).cloned()
}

pub fn query_i64(req: &AppRequest, key: &str) -> Option<i64> {
    req.query.get(key).and_then(|v| v.parse::<i64>().ok())
}

pub fn header_value(req: &AppRequest, key: &str) -> Option<String> {
    req.headers.get(&key.to_ascii_lowercase()).cloned()
}

pub fn body_json<T: DeserializeOwned>(req: &AppRequest) -> crate::Result<T> {
    serde_json::from_slice(&req.body).map_err(|_| ApiError::bad_request("Invalid request body"))
}

pub fn respond(result: crate::Result<AppResponse>) -> AppResponse {
    match result {
        Ok(response) => response,
        Err(error) => AppResponse::from_error(error),
    }
}

pub fn json_response<T: serde::Serialize>(
    status: u16,
    payload: &T,
    link: Option<String>,
) -> AppResponse {
    let mut response = AppResponse::json(status, payload);
    if let Some(link) = link {
        response = response.with_header("Link", &link);
    }
    response
}

pub fn html_response(status: u16, payload: &str) -> AppResponse {
    AppResponse {
        status,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: bytes::Bytes::from(payload.to_string()),
    }
}

pub async fn render_markdown(req: AppRequest, _ctx: &AppContext<'_>) -> AppResponse {
    respond(render_markdown_inner(req).await)
}

async fn render_markdown_inner(req: AppRequest) -> crate::Result<AppResponse> {
    let input: RenderMarkdownInput = body_json(&req)?;
    let _ = (&input.mode, &input.context);
    let html = markdown::render_markdown(&input.text);
    Ok(html_response(200, &html))
}

pub async fn current_user(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    match ctx.user {
        Some(user) => json_response(200, &to_api_user(user), None),
        None => AppResponse::from_error(ApiError::unauthorized()),
    }
}
