use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    fmt::{api as api_fmt, apply_comment_accept, AcceptMode},
    handlers::{body_json, path_i64, path_param},
    router::{AppRequest, AppResponse},
    services,
    types::CreateReactionInput,
    AppContext,
};

use super::respond_native;

#[derive(Debug, Deserialize)]
struct ReactionIdRow {
    id: i64,
}

pub async fn create(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(create_inner(req, ctx).await)
}

async fn create_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let comment_id = path_i64(&req, "id")?;
    let input: CreateReactionInput = body_json(&req)?;

    let (_reaction, created) =
        services::reaction::create_reaction(ctx, &owner, &repo, comment_id, &input).await?;
    let comment = services::comment::get_comment(ctx, &owner, &repo, comment_id).await?;
    let body = api_fmt::to_native_comment(&apply_comment_accept(comment, AcceptMode::Full));
    let status = if created { 201 } else { 200 };
    Ok(AppResponse::json(status, &body.reactions))
}

pub async fn delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(delete_inner(req, ctx).await)
}

async fn delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let comment_id = path_i64(&req, "id")?;
    let content = path_param(&req, "content")?;
    let repo_row = services::repo::get_repo(ctx, &owner, &repo).await?;

    let row = db::query_opt::<ReactionIdRow>(
        ctx.db,
        "SELECT r.id \
         FROM reactions r \
         JOIN comments c ON c.id = r.comment_id \
         WHERE c.repo_id = ?1 AND r.comment_id = ?2 AND r.user_id = ?3 AND r.content = ?4",
        &[
            DbValue::Integer(repo_row.id),
            DbValue::Integer(comment_id),
            DbValue::Integer(user.id),
            DbValue::Text(content),
        ],
    )
    .await?;

    if let Some(reaction) = row {
        services::reaction::delete_reaction(ctx, &owner, &repo, comment_id, reaction.id).await?;
    }

    Ok(AppResponse::no_content())
}
