use serde_json::{Value, json};

use crate::{
    ApiError, AppContext, cookies,
    handlers::{body_json, header_value, path_i64, path_param, query_value},
    router::{AppRequest, AppResponse},
    services,
    types::AuthTokenResponse,
};

use super::respond_native;

pub async fn auth_account(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
            let tokens = services::native::issue_atrium_tokens(ctx, user).await?;
            Ok(with_auth_cookies(
                ctx,
                AppResponse::json(200, &tokens),
                &tokens,
            ))
        }
        .await,
    )
}

pub async fn auth_account_authorize(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let redirect_uri = query_value(&req, "redirect_uri")
                .ok_or_else(|| ApiError::bad_request("missing redirect_uri query parameter"))?;
            let mut callback = url::Url::parse(&format!(
                "{}/api/v1/auth/account/callback",
                ctx.base_url.trim_end_matches('/')
            ))
            .map_err(|_| ApiError::internal("invalid base url"))?;
            callback
                .query_pairs_mut()
                .append_pair("redirect_uri", &redirect_uri);
            if let Some(state) = query_value(&req, "state") {
                callback.query_pairs_mut().append_pair("state", &state);
            }
            let mut login = url::Url::parse(&format!("{}/login", account_base_url(ctx)))
                .map_err(|_| ApiError::internal("invalid account base url"))?;
            login
                .query_pairs_mut()
                .append_pair("return_to", callback.as_str());
            Ok(AppResponse::redirect(login.as_str()))
        }
        .await,
    )
}

pub async fn auth_account_callback(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
            let redirect_uri =
                query_value(&req, "redirect_uri").unwrap_or_else(|| ctx.base_url.to_string());
            let mut location = url::Url::parse(&redirect_uri)
                .map_err(|_| ApiError::bad_request("invalid redirect_uri"))?;
            if let Some(state) = query_value(&req, "state") {
                if !state.is_empty() {
                    location.query_pairs_mut().append_pair("state", &state);
                }
            }
            let mut response = AppResponse::redirect(location.as_str());
            if ctx.jwt_secret.len() >= 16 {
                let tokens = services::native::issue_atrium_tokens(ctx, user).await?;
                response = with_auth_cookies(ctx, response, &tokens);
            }
            Ok(response)
        }
        .await,
    )
}

pub async fn auth_refresh(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let refresh_token = if !req.body.is_empty() {
                let value: Value = body_json(&req)?;
                value
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(ApiError::unauthorized)?
            } else if let Some(header) = req.auth_header.as_deref() {
                crate::auth::bearer_from_header(Some(header))?.ok_or_else(ApiError::unauthorized)?
            } else if let Some(cookie_header) = req.headers.get("cookie") {
                cookies::cookie_value(cookie_header, cookies::REFRESH_COOKIE)
                    .map(str::to_string)
                    .ok_or_else(ApiError::unauthorized)?
            } else {
                return Err(ApiError::unauthorized());
            };
            let tokens = services::native::refresh_atrium_tokens(ctx, &refresh_token).await?;
            Ok(with_auth_cookies(
                ctx,
                AppResponse::json(200, &tokens),
                &tokens,
            ))
        }
        .await,
    )
}

pub async fn auth_session_delete(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            ctx.user.ok_or_else(ApiError::unauthorized)?;
            let secure = cookies::secure_from_base_url(ctx.base_url);
            Ok(AppResponse::no_content()
                .with_header(
                    "Set-Cookie",
                    &cookies::clear_cookie(cookies::ACCESS_COOKIE, secure),
                )
                .with_header(
                    "Set-Cookie",
                    &cookies::clear_cookie(cookies::REFRESH_COOKIE, secure),
                ))
        }
        .await,
    )
}

pub async fn auth_me(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
            Ok(AppResponse::json(
                200,
                &json!({
                    "user": services::native::public_user(user, true),
                    "super_admin": services::native::is_super_admin(ctx).await?,
                }),
            ))
        }
        .await,
    )
}

pub async fn discovery_public_key(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            Ok(AppResponse::json(
                200,
                &services::native::discovery_public_key(ctx).await?,
            ))
        }
        .await,
    )
}

pub async fn docs_discovery(_req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    let html = format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><title>Atrium Discovery 接入指南</title></head><body><h1>Atrium Discovery 接入指南</h1><p>在站点发布 <code>/.well-known/atrium.json</code>，或添加 <code>_atrium.example.com TXT</code>。</p><p>配置 <code>atrium: "v1"</code>、<code>admin_emails</code>，可选填写 origin；不要填写 <code>website_key</code>。</p><p>发现公钥：<code>{}/api/v1/discovery/public-key</code></p></body></html>"#,
        ctx.base_url.trim_end_matches('/')
    );
    AppResponse {
        status: 200,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: bytes::Bytes::from(html),
    }
}

pub async fn create_website(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                201,
                &services::native::create_website(ctx, input).await?,
            ))
        }
        .await,
    )
}

pub async fn list_websites(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    json_result(services::native::list_websites(ctx, &req.query).await, 200)
}

pub async fn get_website(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    let key = path_param(&req, "websiteKey");
    json_result(
        async { services::native::get_website_response(ctx, &key?).await }.await,
        200,
    )
}

pub async fn update_website(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let key = path_param(&req, "websiteKey")?;
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                200,
                &services::native::update_website(ctx, &key, input).await?,
            ))
        }
        .await,
    )
}

pub async fn list_website_admins(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let key = path_param(&req, "websiteKey")?;
            Ok(AppResponse::json(
                200,
                &services::native::list_website_admins(ctx, &key).await?,
            ))
        }
        .await,
    )
}

pub async fn add_website_admin(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let key = path_param(&req, "websiteKey")?;
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                201,
                &services::native::add_website_admin_by_input(ctx, &key, input).await?,
            ))
        }
        .await,
    )
}

pub async fn remove_website_admin(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    no_content(
        async {
            let key = path_param(&req, "websiteKey")?;
            let user_id = path_i64(&req, "userId")?;
            services::native::remove_website_admin(ctx, &key, user_id).await
        }
        .await,
    )
}

pub async fn upsert_page(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let page_key = path_param(&req, "pageKey")?;
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                200,
                &services::native::upsert_page(ctx, &website_key, &page_key, input).await?,
            ))
        }
        .await,
    )
}

pub async fn list_pages(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            Ok(AppResponse::json(
                200,
                &services::native::list_pages(ctx, &website_key, &req.query).await?,
            ))
        }
        .await,
    )
}

pub async fn get_page(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let page_key = path_param(&req, "pageKey")?;
            Ok(AppResponse::json(
                200,
                &services::native::get_page_response(ctx, &website_key, &page_key).await?,
            ))
        }
        .await,
    )
}

pub async fn list_page_comments(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let page_key = path_param(&req, "pageKey")?;
            Ok(AppResponse::json(
                200,
                &services::native::list_page_comments(ctx, &website_key, &page_key, &req.query)
                    .await?,
            ))
        }
        .await,
    )
}

pub async fn create_page_comment(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let page_key = path_param(&req, "pageKey")?;
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                201,
                &services::native::create_page_comment(ctx, &website_key, &page_key, input).await?,
            ))
        }
        .await,
    )
}

pub async fn update_comment(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let id = path_i64(&req, "commentId")?;
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                200,
                &services::native::update_comment(ctx, &website_key, id, input).await?,
            ))
        }
        .await,
    )
}

pub async fn delete_comment(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    no_content(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let id = path_i64(&req, "commentId")?;
            services::native::delete_comment(ctx, &website_key, id).await
        }
        .await,
    )
}

pub async fn set_reaction(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let id = path_i64(&req, "commentId")?;
            let content = path_param(&req, "content")?;
            Ok(AppResponse::json(
                200,
                &services::native::set_comment_reaction(ctx, &website_key, id, &content).await?,
            ))
        }
        .await,
    )
}

pub async fn delete_reaction(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    no_content(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let id = path_i64(&req, "commentId")?;
            let content = path_param(&req, "content")?;
            services::native::delete_comment_reaction(ctx, &website_key, id, &content).await
        }
        .await,
    )
}

pub async fn current_comments(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    let referer = header_value(&req, "referer").or_else(|| header_value(&req, "referrer"));
    json_result(
        services::native::get_current_comments(ctx, referer.as_deref(), &req.query).await,
        200,
    )
}

pub async fn create_current_comment(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let referer = header_value(&req, "referer").or_else(|| header_value(&req, "referrer"));
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                201,
                &services::native::create_current_comment(ctx, referer.as_deref(), input).await?,
            ))
        }
        .await,
    )
}

pub async fn current_replies(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    let referer = header_value(&req, "referer").or_else(|| header_value(&req, "referrer"));
    json_result(
        services::native::list_current_replies(ctx, referer.as_deref(), &req.query).await,
        200,
    )
}

pub async fn set_current_reaction(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let referer = header_value(&req, "referer").or_else(|| header_value(&req, "referrer"));
            let id = path_i64(&req, "commentId")?;
            let content = path_param(&req, "content")?;
            Ok(AppResponse::json(
                200,
                &services::native::set_current_reaction(ctx, referer.as_deref(), id, &content)
                    .await?,
            ))
        }
        .await,
    )
}

pub async fn delete_current_reaction(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    no_content(
        async {
            let referer = header_value(&req, "referer").or_else(|| header_value(&req, "referrer"));
            let id = path_i64(&req, "commentId")?;
            let content = path_param(&req, "content")?;
            services::native::delete_current_reaction(ctx, referer.as_deref(), id, &content).await
        }
        .await,
    )
}

pub async fn moderation_comments(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            Ok(AppResponse::json(
                200,
                &services::native::list_moderation_comments(ctx, &website_key, &req.query).await?,
            ))
        }
        .await,
    )
}

pub async fn ban_user(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let input = body_json(&req)?;
            Ok(AppResponse::json(
                201,
                &services::native::ban_website_user(ctx, &website_key, input).await?,
            ))
        }
        .await,
    )
}

pub async fn list_bans(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            Ok(AppResponse::json(
                200,
                &services::native::list_website_bans(ctx, &website_key).await?,
            ))
        }
        .await,
    )
}

pub async fn unban_user(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    no_content(
        async {
            let website_key = path_param(&req, "websiteKey")?;
            let user_id = path_i64(&req, "userId")?;
            services::native::unban_website_user(ctx, &website_key, user_id).await
        }
        .await,
    )
}

fn with_auth_cookies(
    ctx: &AppContext<'_>,
    mut response: AppResponse,
    tokens: &AuthTokenResponse,
) -> AppResponse {
    let secure = cookies::secure_from_base_url(ctx.base_url);
    response = response.with_header(
        "Set-Cookie",
        &cookies::build_set_cookie(
            cookies::ACCESS_COOKIE,
            &tokens.access_token,
            tokens.expires_in,
            secure,
        ),
    );
    response.with_header(
        "Set-Cookie",
        &cookies::build_set_cookie(
            cookies::REFRESH_COOKIE,
            &tokens.refresh_token,
            30 * 24 * 3600,
            secure,
        ),
    )
}

fn json_result(result: crate::Result<Value>, status: u16) -> AppResponse {
    respond_native(result.map(|value| AppResponse::json(status, &value)))
}

fn no_content(result: crate::Result<()>) -> AppResponse {
    respond_native(result.map(|_| AppResponse::no_content()))
}

fn account_base_url(ctx: &AppContext<'_>) -> String {
    ctx.account_base_url
        .unwrap_or("https://account.jihuayu.com")
        .trim_end_matches('/')
        .to_string()
}
