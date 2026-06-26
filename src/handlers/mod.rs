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
    ApiError, AppContext,
    fmt::user::to_api_user,
    markdown,
    router::{AppRequest, AppResponse},
    types::RenderMarkdownInput,
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
        "Atrium - native website/page/comment service\n",
        "\n",
        "站点接入:\n",
        "  1. 阅读完整接入说明: /docs/discovery\n",
        "  2. 在站点发布 https://<host>/.well-known/atrium.json，或添加 _atrium.<host> TXT\n",
        "  3. 不需要声明 website_key；Atrium 会从当前页面 hostname 推导，origin 可省略\n",
        "  4. admin_emails 里的邮箱登录后会自动认领该 website admin 权限\n",
        "\n",
        "Native API:\n",
        "  POST   /api/v1/auth/account\n",
        "  GET    /api/v1/auth/account/authorize\n",
        "  GET    /api/v1/auth/account/callback\n",
        "  POST   /api/v1/auth/refresh\n",
        "  DELETE /api/v1/auth/session\n",
        "  GET    /api/v1/auth/me\n",
        "  GET    /api/v1/discovery/public-key\n",
        "  POST   /api/v1/websites\n",
        "  GET    /api/v1/websites\n",
        "  GET    /api/v1/websites/{websiteKey}\n",
        "  PATCH  /api/v1/websites/{websiteKey}\n",
        "  GET    /api/v1/websites/{websiteKey}/admins\n",
        "  POST   /api/v1/websites/{websiteKey}/admins\n",
        "  DELETE /api/v1/websites/{websiteKey}/admins/{userId}\n",
        "  PUT    /api/v1/websites/{websiteKey}/pages/{pageKey}\n",
        "  GET    /api/v1/websites/{websiteKey}/pages/{pageKey}/comments\n",
        "  POST   /api/v1/websites/{websiteKey}/pages/{pageKey}/comments\n",
        "  PATCH  /api/v1/websites/{websiteKey}/comments/{commentId}\n",
        "  DELETE /api/v1/websites/{websiteKey}/comments/{commentId}\n",
        "  PUT    /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}\n",
        "  DELETE /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}\n",
        "  GET    /api/v1/comments/current\n",
        "  POST   /api/v1/comments/current\n",
        "  GET    /api/v1/comments/current/replies\n",
        "\n",
        "Source: https://github.com/jihuayu/atrium\n",
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

        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "Link" && value == "</next>; rel=\"next\"")
        );
    }
}
