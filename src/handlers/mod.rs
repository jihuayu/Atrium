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

pub async fn root(_req: AppRequest, _ctx: &AppContext<'_>) -> AppResponse {
    let text = concat!(
        "Atrium - GitHub Issues compatible comment backend\n",
        "\n",
        "GitHub-Compatible API (token auth):\n",
        "  GET    /repos/{owner}/{repo}/issues\n",
        "  POST   /repos/{owner}/{repo}/issues\n",
        "  GET    /repos/{owner}/{repo}/issues/{number}\n",
        "  PATCH  /repos/{owner}/{repo}/issues/{number}\n",
        "  GET    /repos/{owner}/{repo}/issues/{number}/comments\n",
        "  POST   /repos/{owner}/{repo}/issues/{number}/comments\n",
        "  GET    /repos/{owner}/{repo}/issues/comments/{id}\n",
        "  PATCH  /repos/{owner}/{repo}/issues/comments/{id}\n",
        "  DELETE /repos/{owner}/{repo}/issues/comments/{id}\n",
        "  POST   /repos/{owner}/{repo}/issues/comments/{id}/reactions\n",
        "  DELETE /repos/{owner}/{repo}/issues/comments/{id}/reactions/{id}\n",
        "  GET    /search/issues?q=...\n",
        "\n",
        "Native API (JWT auth):\n",
        "  POST   /api/v1/auth/github\n",
        "  POST   /api/v1/auth/google\n",
        "  POST   /api/v1/auth/apple\n",
        "  POST   /api/v1/auth/refresh\n",
        "  DELETE /api/v1/auth/session\n",
        "  GET    /api/v1/auth/me\n",
        "  POST   /api/v1/repos\n",
        "  GET    /api/v1/repos/{owner}/{repo}/threads\n",
        "  POST   /api/v1/repos/{owner}/{repo}/threads\n",
        "  GET    /api/v1/repos/{owner}/{repo}/threads/{number}\n",
        "  PATCH  /api/v1/repos/{owner}/{repo}/threads/{number}\n",
        "  DELETE /api/v1/repos/{owner}/{repo}/threads/{number}\n",
        "  GET    /api/v1/repos/{owner}/{repo}/threads/{number}/comments\n",
        "  POST   /api/v1/repos/{owner}/{repo}/threads/{number}/comments\n",
        "  GET    /api/v1/repos/{owner}/{repo}/comments/{id}\n",
        "  PATCH  /api/v1/repos/{owner}/{repo}/comments/{id}\n",
        "  DELETE /api/v1/repos/{owner}/{repo}/comments/{id}\n",
        "  POST   /api/v1/repos/{owner}/{repo}/comments/{id}/reactions\n",
        "  DELETE /api/v1/repos/{owner}/{repo}/comments/{id}/reactions/{content}\n",
        "  GET    /api/v1/repos/{owner}/{repo}/labels\n",
        "  POST   /api/v1/repos/{owner}/{repo}/labels\n",
        "  DELETE /api/v1/repos/{owner}/{repo}/labels/{name}\n",
        "  GET    /api/v1/repos/{owner}/{repo}/export\n",
        "\n",
        "Source: https://github.com/pnnh/atrium\n",
    );
    AppResponse {
        status: 200,
        headers: vec![(
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: bytes::Bytes::from(text),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_response_adds_link_header_when_present() {
        let response = super::json_response(
            200,
            &serde_json::json!({"ok": true}),
            Some("</next>; rel=\"next\"".to_string()),
        );

        assert!(response
            .headers
            .iter()
            .any(|(name, value)| name == "Link" && value == "</next>; rel=\"next\""));
    }
}
