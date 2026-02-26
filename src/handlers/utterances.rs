use std::collections::HashMap;

use crate::{
    handlers::{header_value, respond},
    router::{AppRequest, AppResponse},
    AppContext,
};

pub async fn proxy_token(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond(proxy_token_inner(req, ctx).await)
}

async fn proxy_token_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let mut headers = HashMap::new();

    // Keep the upstream request shape compatible with utterances' token exchange.
    headers.insert(
        "content-type".to_string(),
        header_value(&req, "content-type").unwrap_or_else(|| "application/json".to_string()),
    );

    for name in [
        "referer",
        "origin",
        "user-agent",
        "cookie",
        "sec-ch-ua",
        "sec-ch-ua-mobile",
        "sec-ch-ua-platform",
    ] {
        if let Some(value) = header_value(&req, name) {
            headers.insert(name.to_string(), value);
        }
    }

    let upstream = ctx.http.post_utterances_token(&req.body, &headers).await?;
    Ok(AppResponse {
        status: upstream.status,
        headers: upstream.headers,
        body: upstream.body,
    })
}
