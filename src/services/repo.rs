use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    types::{GitHubUser, RepoRow},
    AppContext, Result,
};

pub const GLOBAL_OWNER: &str = "_global";

pub async fn ensure_repo(
    ctx: &AppContext<'_>,
    owner: &str,
    repo: &str,
    creator: Option<&GitHubUser>,
) -> Result<RepoRow> {
    if let Some(existing) = find_repo(ctx, owner, repo).await? {
        return Ok(existing);
    }

    if owner.eq_ignore_ascii_case(GLOBAL_OWNER) {
        return Err(ApiError::not_found("Repository"));
    }

    let creator = creator.ok_or_else(|| ApiError::not_found("Repository"))?;
    let has_github = has_provider_identity(ctx, creator.id, "github").await?;
    if !has_github {
        return Err(ApiError::forbidden(
            "Repository does not exist. Create one via POST /api/v1/repos first",
        ));
    }

    let owner_user_id = find_github_user_id_by_login(ctx, owner).await?;
    let admin_user_id = owner_user_id.or(Some(creator.id));
    ctx.db
        .execute(
            "INSERT INTO repos (owner, name, owner_user_id, admin_user_id, issue_counter, created_at) \
             VALUES (?1, ?2, ?3, ?4, 0, datetime('now'))",
            &[
                DbValue::Text(owner.to_string()),
                DbValue::Text(repo.to_string()),
                owner_user_id
                    .map(DbValue::Integer)
                    .unwrap_or(DbValue::Null),
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

pub async fn create_global_repo(
    ctx: &AppContext<'_>,
    actor: &GitHubUser,
    name: &str,
) -> Result<(RepoRow, bool)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::validation("Repository", "name", "missing_field"));
    }

    if let Some(existing) = find_repo(ctx, GLOBAL_OWNER, name).await? {
        if existing.admin_user_id == Some(actor.id) || existing.owner_user_id == Some(actor.id) {
            return Ok((existing, false));
        }
        return Err(ApiError::forbidden("Repository already exists"));
    }

    ctx.db
        .execute(
            "INSERT INTO repos (owner, name, owner_user_id, admin_user_id, issue_counter, created_at) \
             VALUES (?1, ?2, ?3, ?4, 0, datetime('now'))",
            &[
                DbValue::Text(GLOBAL_OWNER.to_string()),
                DbValue::Text(name.to_string()),
                DbValue::Integer(actor.id),
                DbValue::Integer(actor.id),
            ],
        )
        .await?;

    let created = find_repo(ctx, GLOBAL_OWNER, name)
        .await?
        .ok_or_else(|| ApiError::internal("failed to create repository"))?;
    Ok((created, true))
}

async fn find_repo(ctx: &AppContext<'_>, owner: &str, repo: &str) -> Result<Option<RepoRow>> {
    #[derive(Debug, Deserialize)]
    struct Row {
        id: i64,
        owner: String,
        name: String,
        owner_user_id: Option<i64>,
        admin_user_id: Option<i64>,
        issue_counter: i64,
    }

    let row = db::query_opt::<Row>(
        ctx.db,
        "SELECT r.id, r.owner, r.name, r.owner_user_id, r.admin_user_id, r.issue_counter \
         FROM repos r \
         WHERE r.name = ?2 AND ( \
             lower(r.owner) = lower(?1) \
             OR ( \
                 lower(r.owner) <> lower('_global') \
                 AND \
                 r.owner_user_id IS NOT NULL \
                 AND r.owner_user_id = ( \
                     SELECT u.id \
                     FROM users u \
                     JOIN user_identities ui ON ui.user_id = u.id \
                     WHERE ui.provider = 'github' AND lower(u.login) = lower(?1) \
                     LIMIT 1 \
                 ) \
             ) \
         ) \
         ORDER BY CASE WHEN lower(r.owner) = lower(?1) THEN 0 ELSE 1 END, r.id ASC \
         LIMIT 1",
        &[DbValue::Text(owner.to_string()), DbValue::Text(repo.to_string())],
    )
    .await?;

    Ok(row.map(|v| RepoRow {
        id: v.id,
        owner: v.owner,
        name: v.name,
        owner_user_id: v.owner_user_id,
        admin_user_id: v.admin_user_id,
        issue_counter: v.issue_counter,
    }))
}

async fn has_provider_identity(ctx: &AppContext<'_>, user_id: i64, provider: &str) -> Result<bool> {
    #[derive(Debug, Deserialize)]
    struct HitRow {
        #[serde(rename = "hit")]
        _hit: i64,
    }

    let hit = db::query_opt::<HitRow>(
        ctx.db,
        "SELECT 1 AS hit FROM user_identities WHERE user_id = ?1 AND provider = ?2 LIMIT 1",
        &[
            DbValue::Integer(user_id),
            DbValue::Text(provider.to_string()),
        ],
    )
    .await?;
    Ok(hit.is_some())
}

async fn find_github_user_id_by_login(ctx: &AppContext<'_>, login: &str) -> Result<Option<i64>> {
    #[derive(Debug, Deserialize)]
    struct IdRow {
        id: i64,
    }

    let row = db::query_opt::<IdRow>(
        ctx.db,
        "SELECT u.id \
         FROM users u \
         JOIN user_identities ui ON ui.user_id = u.id \
         WHERE ui.provider = 'github' AND lower(u.login) = lower(?1) \
         LIMIT 1",
        &[DbValue::Text(login.to_string())],
    )
    .await?;
    Ok(row.map(|v| v.id))
}
