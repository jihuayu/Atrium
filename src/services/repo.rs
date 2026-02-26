use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    types::{GitHubUser, RepoRow},
    AppContext, Result,
};

pub async fn ensure_repo(
    ctx: &AppContext<'_>,
    owner: &str,
    repo: &str,
    creator: Option<&GitHubUser>,
) -> Result<RepoRow> {
    if let Some(existing) = find_repo(ctx, owner, repo).await? {
        return Ok(existing);
    }

    let admin_user_id = creator.map(|v| v.id);
    ctx.db
        .execute(
            "INSERT INTO repos (owner, name, admin_user_id, issue_counter, created_at) VALUES (?1, ?2, ?3, 0, datetime('now'))",
            &[
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo.to_string()),
                admin_user_id.map(DbValue::Integer).unwrap_or(DbValue::Null),
            ],
        )
        .await?;

    find_repo(ctx, owner, repo)
        .await?
        .ok_or_else(|| ApiError::internal("failed to create repo"))
}

pub async fn get_repo(ctx: &AppContext<'_>, owner: &str, repo: &str) -> Result<RepoRow> {
    find_repo(ctx, owner, repo)
        .await?
        .ok_or_else(|| ApiError::not_found("Repository"))
}

async fn find_repo(ctx: &AppContext<'_>, owner: &str, repo: &str) -> Result<Option<RepoRow>> {
    #[derive(Debug, Deserialize)]
    struct Row {
        id: i64,
        owner: String,
        name: String,
        admin_user_id: Option<i64>,
        issue_counter: i64,
    }

    let row = db::query_opt::<Row>(
        ctx.db,
            "SELECT id, owner, name, admin_user_id, issue_counter FROM repos WHERE owner = ?1 AND name = ?2",
            &[DbValue::Text(owner.to_string()), DbValue::Text(repo.to_string())],
        )
    .await?;

    Ok(row.map(|v| RepoRow {
        id: v.id,
        owner: v.owner,
        name: v.name,
        admin_user_id: v.admin_user_id,
        issue_counter: v.issue_counter,
    }))
}
