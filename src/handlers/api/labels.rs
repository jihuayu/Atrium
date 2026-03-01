use crate::{
    db::DbValue,
    error::ApiError,
    handlers::{body_json, path_param},
    router::{AppRequest, AppResponse},
    services,
    types::{CreateLabelInput, NativeLabel},
    AppContext,
};

use super::respond_native;

pub async fn list(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(list_inner(req, ctx).await)
}

async fn list_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;

    let labels = services::label::list_labels(ctx, &owner, &repo).await?;
    let data: Vec<_> = labels
        .into_iter()
        .map(|label| NativeLabel {
            id: label.id,
            name: label.name,
            color: label.color,
        })
        .collect();
    Ok(AppResponse::json(200, &data))
}

pub async fn create(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(create_inner(req, ctx).await)
}

async fn create_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    ensure_repo_admin(ctx, &owner, &repo).await?;

    let input: CreateLabelInput = body_json(&req)?;
    let label = services::label::create_label(ctx, &owner, &repo, &input).await?;
    Ok(AppResponse::json(
        201,
        &NativeLabel {
            id: label.id,
            name: label.name,
            color: label.color,
        },
    ))
}

pub async fn delete(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(delete_inner(req, ctx).await)
}

async fn delete_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let name = path_param(&req, "name")?;
    let repo_row = ensure_repo_admin(ctx, &owner, &repo).await?;

    ctx.db
        .execute(
            "DELETE FROM labels WHERE repo_id = ?1 AND name = ?2",
            &[DbValue::Integer(repo_row.id), DbValue::Text(name)],
        )
        .await?;

    Ok(AppResponse::no_content())
}

async fn ensure_repo_admin(
    ctx: &AppContext<'_>,
    owner: &str,
    repo: &str,
) -> crate::Result<crate::types::RepoRow> {
    let actor = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let repo_row = services::repo::get_repo(ctx, owner, repo).await?;
    if repo_row.admin_user_id != Some(actor.id) {
        return Err(ApiError::forbidden("Admin required"));
    }
    Ok(repo_row)
}
