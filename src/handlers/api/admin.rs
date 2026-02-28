use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    handlers::{body_json, path_param},
    router::{AppRequest, AppResponse},
    services,
    types::{NativeRepoSettings, UpdateRepoSettingsInput},
    AppContext,
};

use super::respond_native;

pub async fn get(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(get_inner(req, ctx).await)
}

async fn get_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let repo_row = ensure_repo_admin(ctx, &owner, &repo).await?;

    Ok(AppResponse::json(
        200,
        &NativeRepoSettings {
            owner: repo_row.owner,
            name: repo_row.name,
            admin_user_id: repo_row.admin_user_id,
            issue_counter: repo_row.issue_counter,
        },
    ))
}

pub async fn update(req: AppRequest, ctx: &AppContext<'_>) -> AppResponse {
    respond_native(update_inner(req, ctx).await)
}

async fn update_inner(req: AppRequest, ctx: &AppContext<'_>) -> crate::Result<AppResponse> {
    let owner = path_param(&req, "owner")?;
    let repo = path_param(&req, "repo")?;
    let current = ensure_repo_admin(ctx, &owner, &repo).await?;
    let input: UpdateRepoSettingsInput = body_json(&req)?;

    if let Some(admin_user_id) = input.admin_user_id {
        ensure_user_exists(ctx, admin_user_id).await?;
        ctx.db
            .execute(
                "UPDATE repos SET admin_user_id = ?1 WHERE id = ?2",
                &[
                    DbValue::Integer(admin_user_id),
                    DbValue::Integer(current.id),
                ],
            )
            .await?;
    }

    let updated = services::repo::get_repo(ctx, &owner, &repo).await?;
    Ok(AppResponse::json(
        200,
        &NativeRepoSettings {
            owner: updated.owner,
            name: updated.name,
            admin_user_id: updated.admin_user_id,
            issue_counter: updated.issue_counter,
        },
    ))
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

async fn ensure_user_exists(ctx: &AppContext<'_>, user_id: i64) -> crate::Result<()> {
    #[derive(Debug, Deserialize)]
    struct UserIdRow {
        _id: i64,
    }

    let user = db::query_opt::<UserIdRow>(
        ctx.db,
        "SELECT id FROM users WHERE id = ?1",
        &[DbValue::Integer(user_id)],
    )
    .await?;
    if user.is_none() {
        return Err(ApiError::validation(
            "Repository",
            "admin_user_id",
            "invalid",
        ));
    }
    Ok(())
}
