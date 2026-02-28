use crate::{
    auth::{bearer_from_header, resolve_github_user, resolve_xtalk_jwt_user},
    handlers::body_json,
    jwks,
    router::{AppRequest, AppResponse},
    services,
    types::{ProviderTokenInput, RefreshTokenInput},
    ApiError, AppContext,
};

use super::respond_native;

const TOKEN_CACHE_TTL_SECS: i64 = 3600;

pub async fn github(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(github_inner(req, ctx).await)
}

async fn github_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let input: ProviderTokenInput = body_json(&req)?;
    let user = resolve_github_user(ctx.db, ctx.http, &input.token, TOKEN_CACHE_TTL_SECS).await?;
    let tokens = services::auth::issue_xtalk_jwt(ctx, &user).await?;
    Ok(AppResponse::json(200, &tokens))
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
    Ok(AppResponse::json(200, &tokens))
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
    Ok(AppResponse::json(200, &tokens))
}

pub async fn refresh(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(refresh_inner(req, ctx).await)
}

async fn refresh_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let refresh_token = if req.body.is_empty() {
        bearer_from_header(req.auth_header.as_deref())?.ok_or_else(ApiError::unauthorized)?
    } else {
        let input: RefreshTokenInput = body_json(&req)?;
        input.refresh_token
    };
    let tokens = services::auth::refresh_xtalk_jwt(ctx, &refresh_token).await?;
    Ok(AppResponse::json(200, &tokens))
}

pub async fn session_delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(session_delete_inner(req, ctx).await)
}

async fn session_delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let user = if let Some(user) = ctx.user {
        user.clone()
    } else {
        resolve_xtalk_jwt_user(ctx.db, req.auth_header.as_deref(), ctx.jwt_secret)
            .await?
            .ok_or_else(ApiError::unauthorized)?
    };
    services::auth::revoke_current_session(ctx, user.id).await?;
    Ok(AppResponse::no_content())
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
